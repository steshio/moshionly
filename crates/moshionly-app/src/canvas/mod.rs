//! Pictures of the artboards, drawn off the UI thread.
//!
//! [`CanvasRenderer`] evaluates and renders artboards on worker threads and
//! turns the results into textures for the viewport. The UI thread only
//! asks: [`request`](CanvasRenderer::request) says what an artboard should
//! look like now, and [`poll`](CanvasRenderer::poll) picks up whatever has
//! finished. Until a new picture arrives the previous one stays on screen,
//! stretched to fit, so the window never waits on a render.
//!
//! Each artboard has at most one render waiting. A newer request takes the
//! place of a waiting one, which is then never drawn, and a finished render
//! that is older than the picture on screen is dropped. When nothing
//! changes, nothing is requested and the window stays idle.

pub mod plan;

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use egui::{ColorImage, Context, Rect, TextureHandle, TextureOptions, pos2, vec2};
use moshionly_core::scene::Content;
use moshionly_core::{Document, NodeId, Scene, SceneNode, evaluate, evaluate_at_rest};
use moshionly_render::{RenderOptions, Renderer};

pub use self::plan::{MAX_TEXTURE_SIDE, Plan, density, pixel_size, plan};

/// Everything that decides the pixels of a picture of an artboard.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderKey {
    /// The session revision of the document.
    pub revision: u64,
    /// The moment shown, in milliseconds, or `None` for the design at rest.
    pub time: Option<f64>,
    /// The density and the part of the artboard.
    pub plan: Plan,
    /// A layer left out of the picture, such as text being edited on the
    /// canvas, which the editor draws itself.
    pub skip: Option<NodeId>,
}

/// A rendered artboard, ready to draw.
pub struct Picture {
    /// The texture.
    pub texture: TextureHandle,
    /// The part of the artboard the texture covers, in the artboard's own
    /// coordinates.
    pub region: Rect,
    /// What the picture shows.
    pub key: RenderKey,
    generation: u64,
}

/// How long renders took, for the most recent ones.
#[derive(Clone, Debug, Default)]
pub struct RenderTimes {
    recent: VecDeque<Duration>,
}

impl RenderTimes {
    const KEPT: usize = 64;

    fn record(&mut self, elapsed: Duration) {
        if self.recent.len() == Self::KEPT {
            self.recent.pop_front();
        }
        self.recent.push_back(elapsed);
    }

    /// The most recent render's time.
    pub fn last(&self) -> Option<Duration> {
        self.recent.back().copied()
    }

    /// The mean of the recent renders' times.
    pub fn average(&self) -> Option<Duration> {
        let count = u32::try_from(self.recent.len()).ok().filter(|n| *n > 0)?;
        Some(self.recent.iter().sum::<Duration>() / count)
    }

    /// How many renders the figures cover.
    pub fn count(&self) -> usize {
        self.recent.len()
    }
}

/// One render to do.
struct Job {
    artboard: NodeId,
    document: Arc<Document>,
    key: RenderKey,
    generation: u64,
}

/// One render done.
struct Done {
    artboard: NodeId,
    key: RenderKey,
    generation: u64,
    region: Rect,
    image: Result<ColorImage, String>,
    elapsed: Duration,
}

#[derive(Default)]
struct Queue {
    /// Waiting renders, at most one per artboard, oldest first.
    waiting: VecDeque<Job>,
    /// Renders being drawn right now.
    busy: usize,
    shutdown: bool,
}

struct Shared {
    renderer: Renderer,
    queue: Mutex<Queue>,
    /// Wakes a worker when a job arrives, and a waiter when all are done.
    signal: Condvar,
    done: Mutex<Vec<Done>>,
    ctx: Mutex<Option<Context>>,
}

impl Shared {
    fn queue(&self) -> MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn done(&self) -> MutexGuard<'_, Vec<Done>> {
        self.done.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn wake_window(&self) {
        let ctx = self.ctx.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(ctx) = ctx.as_ref() {
            ctx.request_repaint();
        }
    }
}

/// Renders artboards for the canvas. See the [module docs](self).
pub struct CanvasRenderer {
    shared: Arc<Shared>,
    workers: Vec<JoinHandle<()>>,
    /// Render on the calling thread instead, for tests and screenshots.
    inline: bool,
    pictures: HashMap<NodeId, Picture>,
    /// The latest request for each artboard.
    requested: HashMap<NodeId, RenderKey>,
    generation: u64,
    times: RenderTimes,
    last_error: Option<String>,
}

impl CanvasRenderer {
    /// A canvas renderer drawing with `renderer` on a few worker threads.
    pub fn new(renderer: Renderer) -> Self {
        let threads = std::thread::available_parallelism()
            .map_or(2, |n| n.get().saturating_sub(1))
            .clamp(1, 3);
        let mut canvas = Self::build(renderer, false);
        for index in 0..threads {
            let shared = Arc::clone(&canvas.shared);
            let spawned = std::thread::Builder::new()
                .name(format!("moshionly-canvas-{index}"))
                .spawn(move || work(&shared));
            match spawned {
                Ok(handle) => canvas.workers.push(handle),
                Err(_) => break,
            }
        }
        // Without a single thread, draw on the caller's rather than never.
        canvas.inline = canvas.workers.is_empty();
        canvas
    }

    /// A canvas renderer that draws on the calling thread, inside
    /// [`request`](Self::request). The window waits for every render; use
    /// it for tests and screenshots, never in the app.
    pub fn inline(renderer: Renderer) -> Self {
        Self::build(renderer, true)
    }

    fn build(renderer: Renderer, inline: bool) -> Self {
        Self {
            shared: Arc::new(Shared {
                renderer,
                queue: Mutex::new(Queue::default()),
                signal: Condvar::new(),
                done: Mutex::new(Vec::new()),
                ctx: Mutex::new(None),
            }),
            workers: Vec::new(),
            inline,
            pictures: HashMap::new(),
            requested: HashMap::new(),
            generation: 0,
            times: RenderTimes::default(),
            last_error: None,
        }
    }

    /// Asks for a picture of `artboard` as `key` describes. Nothing happens
    /// when that is what was asked for last time.
    pub fn request(&mut self, artboard: &NodeId, document: &Arc<Document>, key: RenderKey) {
        if self.requested.get(artboard) == Some(&key) {
            return;
        }
        self.requested.insert(artboard.clone(), key.clone());
        self.generation += 1;
        let job = Job {
            artboard: artboard.clone(),
            document: Arc::clone(document),
            key,
            generation: self.generation,
        };
        if self.inline {
            let done = render(&self.shared.renderer, job);
            self.shared.done().push(done);
            return;
        }
        let mut queue = self.shared.queue();
        // A newer request takes the place of one still waiting.
        queue
            .waiting
            .retain(|waiting| waiting.artboard != *artboard);
        queue.waiting.push_back(job);
        drop(queue);
        self.shared.signal.notify_all();
    }

    /// Picks up finished renders and turns them into textures. Call once a
    /// frame; it also tells the workers which window to wake.
    pub fn poll(&mut self, ctx: &Context) {
        {
            let mut slot = self
                .shared
                .ctx
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if slot.is_none() {
                *slot = Some(ctx.clone());
            }
        }
        let finished = std::mem::take(&mut *self.shared.done());
        for done in finished {
            self.times.record(done.elapsed);
            let image = match done.image {
                Ok(image) => image,
                Err(message) => {
                    self.last_error = Some(message);
                    continue;
                }
            };
            let newer = self
                .pictures
                .get(&done.artboard)
                .is_none_or(|picture| done.generation > picture.generation);
            if !newer {
                continue;
            }
            match self.pictures.get_mut(&done.artboard) {
                Some(picture) => {
                    picture.texture.set(image, TextureOptions::LINEAR);
                    picture.region = done.region;
                    picture.key = done.key;
                    picture.generation = done.generation;
                }
                None => {
                    let texture = ctx.load_texture(
                        format!("artboard-{}", done.artboard),
                        image,
                        TextureOptions::LINEAR,
                    );
                    self.pictures.insert(
                        done.artboard,
                        Picture {
                            texture,
                            region: done.region,
                            key: done.key,
                            generation: done.generation,
                        },
                    );
                }
            }
        }
    }

    /// The picture of `artboard` on screen now, which may be older than the
    /// latest request.
    pub fn picture(&self, artboard: &NodeId) -> Option<&Picture> {
        self.pictures.get(artboard)
    }

    /// The latest request for `artboard`.
    pub fn requested(&self, artboard: &NodeId) -> Option<&RenderKey> {
        self.requested.get(artboard)
    }

    /// Forgets the pictures of artboards that are not in `document`.
    pub fn retain(&mut self, document: &Document) {
        self.pictures.retain(|id, _| document.contains(id));
        self.requested.retain(|id, _| document.contains(id));
    }

    /// Forgets every picture, for example when another document is opened.
    pub fn clear(&mut self) {
        self.pictures.clear();
        self.requested.clear();
        self.shared.queue().waiting.clear();
    }

    /// Whether no render is waiting or being drawn.
    pub fn is_idle(&self) -> bool {
        let queue = self.shared.queue();
        queue.waiting.is_empty() && queue.busy == 0
    }

    /// Waits until no render is waiting or being drawn, at most `timeout`.
    /// Returns whether everything finished. For tests.
    pub fn wait_idle(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut queue = self.shared.queue();
        while !(queue.waiting.is_empty() && queue.busy == 0) {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            queue = self
                .shared
                .signal
                .wait_timeout(queue, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        true
    }

    /// How long recent renders took.
    pub fn times(&self) -> &RenderTimes {
        &self.times
    }

    /// Why the most recent failed render failed.
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }
}

impl Drop for CanvasRenderer {
    fn drop(&mut self) {
        self.shared.queue().shutdown = true;
        self.shared.signal.notify_all();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

/// A worker: takes jobs until the renderer is dropped.
fn work(shared: &Shared) {
    loop {
        let job = {
            let mut queue = shared.queue();
            loop {
                if queue.shutdown {
                    return;
                }
                if let Some(job) = queue.waiting.pop_front() {
                    queue.busy += 1;
                    break job;
                }
                queue = shared
                    .signal
                    .wait(queue)
                    .unwrap_or_else(PoisonError::into_inner);
            }
        };
        let done = render(&shared.renderer, job);
        shared.done().push(done);
        shared.queue().busy -= 1;
        shared.signal.notify_all();
        shared.wake_window();
    }
}

/// Evaluates and draws one job.
fn render(renderer: &Renderer, job: Job) -> Done {
    let started = Instant::now();
    let (region, image) = match draw(renderer, &job) {
        Ok((region, image)) => (region, Ok(image)),
        Err(message) => (Rect::NOTHING, Err(message)),
    };
    Done {
        artboard: job.artboard,
        key: job.key,
        generation: job.generation,
        region,
        image,
        elapsed: started.elapsed(),
    }
}

/// The scene of an artboard at a moment, or at rest.
pub fn scene_of(
    renderer: &Renderer,
    document: &Document,
    artboard: &NodeId,
    time: Option<f64>,
) -> Result<Scene, String> {
    let fonts = renderer.fonts();
    match time {
        Some(time) => evaluate(document, artboard, time, fonts),
        None => evaluate_at_rest(document, artboard, fonts),
    }
    .map_err(|error| error.to_string())
}

/// Evaluates and renders what `job` asks for: the region it covers and the
/// pixels.
fn draw(renderer: &Renderer, job: &Job) -> Result<(Rect, ColorImage), String> {
    let mut scene = scene_of(renderer, &job.document, &job.artboard, job.key.time)?;
    if let Some(skip) = &job.key.skip {
        leave_out(&mut scene.nodes, skip);
    }
    let mut options = RenderOptions::new().with_scale(f64::from(job.key.plan.scale));
    let region = match job.key.plan.region {
        Some(region) => {
            options = options.with_region(moshionly_core::Rect::new(
                f64::from(region.min.x),
                f64::from(region.min.y),
                f64::from(region.width()),
                f64::from(region.height()),
            ));
            region
        }
        None => Rect::from_min_size(
            pos2(0.0, 0.0),
            vec2(scene.width as f32, scene.height as f32),
        ),
    };
    let frame = renderer
        .render(&scene, &options)
        .map_err(|error| error.to_string())?;
    let size = [frame.width() as usize, frame.height() as usize];
    Ok((
        region,
        ColorImage::from_rgba_premultiplied(size, frame.premultiplied()),
    ))
}

/// Removes every node made for the layer `id` from a scene.
fn leave_out(nodes: &mut Vec<SceneNode>, id: &NodeId) {
    nodes.retain(|node| node.id != *id);
    for node in nodes {
        if let Content::Group { children } = &mut node.content {
            leave_out(children, id);
        }
    }
}

#[cfg(test)]
mod tests {
    use moshionly_core::io;
    use moshionly_render::{AssetLoader, FontLibrary};

    use super::*;

    fn document() -> Arc<Document> {
        Arc::new(
            io::parse(
                r##"{"format":"moshionly","version":1,"name":"Test","nodes":[
                {"id":"ab","parent":null,"index":"a0","type":"artboard","width":200,"height":100,"fill":"#FFFFFF"},
                {"id":"box","parent":"ab","index":"a0","type":"rect","x":20,"y":20,"width":40,"height":40,"fill":"#FF0000"},
                {"id":"fade","parent":"ab","index":"a0","type":"fade_in","target":"box","start":0,"end":500}
                ]}"##,
            )
            .unwrap(),
        )
    }

    fn renderer() -> Renderer {
        Renderer::new(FontLibrary::bundled(), AssetLoader::default())
    }

    fn key(revision: u64, time: Option<f64>, scale: f32) -> RenderKey {
        RenderKey {
            revision,
            time,
            plan: Plan {
                scale,
                region: None,
            },
            skip: None,
        }
    }

    #[test]
    fn renders_arrive_as_textures() {
        let ctx = Context::default();
        let document = document();
        let artboard = NodeId::from("ab");
        let mut canvas = CanvasRenderer::new(renderer());
        canvas.request(&artboard, &document, key(1, None, 0.5));
        assert!(canvas.wait_idle(Duration::from_secs(30)));
        canvas.poll(&ctx);
        let picture = canvas.picture(&artboard).expect("a picture");
        assert_eq!(picture.texture.size(), [100, 50]);
        assert_eq!(picture.region.size(), vec2(200.0, 100.0));
        assert_eq!(canvas.times().count(), 1);
        assert!(canvas.last_error().is_none());
    }

    #[test]
    fn the_same_request_twice_renders_once() {
        let ctx = Context::default();
        let document = document();
        let artboard = NodeId::from("ab");
        let mut canvas = CanvasRenderer::inline(renderer());
        canvas.request(&artboard, &document, key(1, None, 1.0));
        canvas.request(&artboard, &document, key(1, None, 1.0));
        canvas.poll(&ctx);
        assert_eq!(canvas.times().count(), 1);
        // A new time is a new picture.
        canvas.request(&artboard, &document, key(1, Some(0.0), 1.0));
        canvas.poll(&ctx);
        assert_eq!(canvas.times().count(), 2);
        assert_eq!(canvas.picture(&artboard).unwrap().key.time, Some(0.0));
    }

    #[test]
    fn older_renders_never_replace_newer_ones() {
        let ctx = Context::default();
        let document = document();
        let artboard = NodeId::from("ab");
        let mut canvas = CanvasRenderer::inline(renderer());
        canvas.request(&artboard, &document, key(1, None, 1.0));
        canvas.request(&artboard, &document, key(2, None, 1.0));
        // Both finished; deliver them newest first.
        canvas.shared.done().reverse();
        canvas.poll(&ctx);
        assert_eq!(canvas.picture(&artboard).unwrap().key.revision, 2);
    }

    #[test]
    fn a_waiting_render_is_replaced_by_a_newer_request() {
        let document = document();
        let artboard = NodeId::from("ab");
        // No workers: jobs stay in the queue where they can be inspected.
        let mut canvas = CanvasRenderer::build(renderer(), false);
        canvas.request(&artboard, &document, key(1, None, 1.0));
        canvas.request(&artboard, &document, key(2, None, 1.0));
        canvas.request(&NodeId::from("other"), &document, key(2, None, 1.0));
        let queue = canvas.shared.queue();
        let waiting: Vec<u64> = queue.waiting.iter().map(|job| job.key.revision).collect();
        assert_eq!(waiting, [2, 2]);
        assert_eq!(queue.waiting[0].artboard, artboard);
    }

    #[test]
    fn a_skipped_layer_is_left_out_of_the_picture() {
        let document = document();
        let job = |skip: Option<&str>| Job {
            artboard: NodeId::from("ab"),
            document: Arc::clone(&document),
            key: RenderKey {
                skip: skip.map(NodeId::from),
                ..key(1, None, 1.0)
            },
            generation: 1,
        };
        let pixel = |image: &ColorImage| image.pixels[40 * 200 + 40];
        let (region, with) = draw(&renderer(), &job(None)).unwrap();
        let (_, without) = draw(&renderer(), &job(Some("box"))).unwrap();
        assert_eq!(region.size(), vec2(200.0, 100.0));
        assert_eq!(pixel(&with), egui::Color32::RED);
        assert_eq!(pixel(&without), egui::Color32::WHITE);
    }

    #[test]
    fn a_broken_render_keeps_the_old_picture() {
        let ctx = Context::default();
        let document = document();
        let artboard = NodeId::from("ab");
        let mut canvas = CanvasRenderer::inline(renderer());
        canvas.request(&artboard, &document, key(1, None, 1.0));
        canvas.poll(&ctx);
        // An artboard that does not exist cannot be evaluated.
        canvas.request(&NodeId::from("missing"), &document, key(1, None, 1.0));
        canvas.poll(&ctx);
        assert!(canvas.last_error().is_some());
        assert!(canvas.picture(&artboard).is_some());
        assert!(canvas.picture(&NodeId::from("missing")).is_none());
        canvas.retain(&Document::new("Empty"));
        assert!(canvas.picture(&artboard).is_none());
    }
}
