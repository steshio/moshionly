//! The renderer: a scene and options in, a frame out.

use moshionly_core::{NodeId, Point, Scene};
use skia_safe as sk;

use crate::assets::AssetLoader;
use crate::draw::Painter;
use crate::error::{RenderError, RenderWarning};
use crate::fonts::FontLibrary;
use crate::frame::{BYTES_PER_PIXEL, Frame};
use crate::hit::{Geometry, HIT_TOLERANCE, LayerBounds};
use crate::options::{Background, RenderOptions};

/// Draws scenes.
///
/// A renderer keeps what is worth keeping between frames, so make one and
/// use it for every frame. It can be created on one thread and used on
/// another, and shared between threads.
#[derive(Debug, Clone)]
pub struct Renderer {
    fonts: FontLibrary,
    assets: AssetLoader,
}

impl Renderer {
    /// A renderer that draws text with `fonts` and images from `assets`.
    ///
    /// Evaluate scenes with the same font library as the measurer (it is
    /// one), so that text is split where it is drawn.
    pub fn new(fonts: FontLibrary, assets: AssetLoader) -> Self {
        Self { fonts, assets }
    }

    /// The fonts text is drawn with. They are also the text measurer to
    /// evaluate scenes with.
    pub fn fonts(&self) -> &FontLibrary {
        &self.fonts
    }

    /// Where images come from.
    pub fn assets(&self) -> &AssetLoader {
        &self.assets
    }

    /// Renders a scene into a new frame.
    ///
    /// The frame is `options.scale` times the size of the artboard, or of
    /// `options.region` when that is set. See [`RenderOptions::frame_size`].
    pub fn render(&self, scene: &Scene, options: &RenderOptions) -> Result<Frame, RenderError> {
        let (width, height) = options.frame_size(scene)?;
        let mut frame = Frame::new(width, height)?;
        self.render_to(scene, options, &mut frame)?;
        Ok(frame)
    }

    /// Renders a scene into `frame`, reusing its memory when it already
    /// has the size this render needs. For playback and export, which draw
    /// frame after frame of the same size, this saves allocating and
    /// clearing a new buffer each time.
    ///
    /// On error the frame is left as it was.
    pub fn render_to(
        &self,
        scene: &Scene,
        options: &RenderOptions,
        frame: &mut Frame,
    ) -> Result<(), RenderError> {
        let (width, height) = options.frame_size(scene)?;
        if (frame.width(), frame.height()) == (width, height) {
            frame.clear();
        } else {
            *frame = Frame::new(width, height)?;
        }
        let region = options.region_of(scene);
        let info = sk::ImageInfo::new(
            (width as i32, height as i32),
            sk::ColorType::RGBA8888,
            sk::AlphaType::Premul,
            None,
        );
        let row_bytes = width as usize * BYTES_PER_PIXEL;
        let warnings = {
            let mut surface =
                sk::surfaces::wrap_pixels(&info, frame.premultiplied_mut(), row_bytes, None)
                    .ok_or(RenderError::Surface { width, height })?;
            let canvas = surface.canvas();
            canvas.scale((options.scale as f32, options.scale as f32));
            canvas.translate((-region.x as f32, -region.y as f32));
            self.draw(scene, canvas, options.background)
        };
        frame.set_warnings(warnings);
        Ok(())
    }

    /// Draws a scene onto a canvas the caller owns, on the artboard's own
    /// fill.
    ///
    /// The artboard's top-left corner goes where the canvas's current
    /// transform puts the origin, and one artboard pixel is one unit, so
    /// scale and translate the canvas first to place the scene. The canvas
    /// comes back with the transform and clip it had.
    ///
    /// Returns what was drawn differently from what the scene asked for.
    pub fn render_into(&self, scene: &Scene, canvas: &sk::Canvas) -> Vec<RenderWarning> {
        self.draw(scene, canvas, Background::Artboard)
    }

    /// The layers under `point`, in artboard coordinates, frontmost first.
    ///
    /// A layer is under the point when what it draws is: its fill, its
    /// stroke (thin strokes count within [`HIT_TOLERANCE`] artboard
    /// pixels), its image, or for text its box. Transforms, clips and masks
    /// are respected, and a shape that draws nothing is found by its
    /// outline so it can still be selected. Groups are not listed for
    /// themselves, only through a background or outline of their own; find
    /// the group a layer belongs to in the document. Each id appears once.
    pub fn hit_test(&self, scene: &Scene, point: Point) -> Vec<NodeId> {
        self.hit_test_within(scene, point, HIT_TOLERANCE)
    }

    /// [`hit_test`](Self::hit_test) with thin strokes counting within
    /// `tolerance` artboard pixels. An editor zoomed to 4x might pass a
    /// quarter of the default to keep the reach the same on screen.
    pub fn hit_test_within(&self, scene: &Scene, point: Point, tolerance: f64) -> Vec<NodeId> {
        self.geometry().hit_test(scene, point, tolerance)
    }

    /// Where the layer `id` is in `scene`: its box's corners and the box
    /// around what it draws, in artboard coordinates. `None` when the
    /// layer is not in the scene (hidden at that moment, for example).
    pub fn bounds_of(&self, scene: &Scene, id: &NodeId) -> Option<LayerBounds> {
        self.geometry().bounds_of(scene, id)
    }

    fn geometry(&self) -> Geometry<'_> {
        Geometry { fonts: &self.fonts }
    }

    fn draw(
        &self,
        scene: &Scene,
        canvas: &sk::Canvas,
        background: Background,
    ) -> Vec<RenderWarning> {
        let mut painter = Painter::new(canvas, &self.fonts, &self.assets);
        painter.scene(scene, background);
        painter.into_warnings()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn renderer() -> Renderer {
        Renderer::new(FontLibrary::bundled(), AssetLoader::default())
    }
    use moshionly_core::model::{BlendMode, GradientStop, ImageFit, Shadow};
    use moshionly_core::scene::{Clip, Content, SceneNode, ScenePaint};
    use moshionly_core::{Affine, Color, NodeId, Point, Rect};

    fn scene(background: Option<ScenePaint>) -> Scene {
        Scene {
            artboard: NodeId::from("ab"),
            time_ms: 0.0,
            width: 40.0,
            height: 20.0,
            background,
            nodes: Vec::new(),
        }
    }

    #[test]
    fn the_background_follows_the_options() {
        let renderer = renderer();
        let blue = Color::rgb(0, 0, 255);
        let filled = scene(Some(ScenePaint::solid(blue)));
        let options = RenderOptions::new();

        let frame = renderer.render(&filled, &options).unwrap();
        assert_eq!((frame.width(), frame.height()), (40, 20));
        assert!(frame.is_opaque());
        assert_eq!(frame.premultiplied_pixel(39, 19), Some([0, 0, 255, 255]));
        assert!(frame.warnings().is_empty());

        let clear = renderer
            .render(&filled, &options.with_background(Background::Transparent))
            .unwrap();
        assert_eq!(clear.premultiplied_pixel(5, 5), Some([0, 0, 0, 0]));

        // An artboard without a fill is transparent, unless a colour is put
        // behind it.
        let empty = scene(None);
        let frame = renderer.render(&empty, &options).unwrap();
        assert_eq!(frame.premultiplied_pixel(5, 5), Some([0, 0, 0, 0]));
        let over = options.with_background(Background::Over(Color::WHITE));
        let frame = renderer.render(&empty, &over).unwrap();
        assert_eq!(frame.premultiplied_pixel(5, 5), Some([255, 255, 255, 255]));

        // A see-through fill lets the colour behind it show.
        let glass = scene(Some(ScenePaint::solid(Color::rgba(0, 0, 0, 128))));
        let frame = renderer.render(&glass, &over).unwrap();
        assert!(frame.is_opaque());
        assert_eq!(frame.premultiplied_pixel(5, 5), Some([127, 127, 127, 255]));
    }

    #[test]
    fn scale_and_region_choose_the_pixels() {
        let renderer = renderer();
        // Black on the left edge, white on the right edge.
        let gradient = scene(Some(ScenePaint::LinearGradient {
            start: Point::new(0.0, 0.0),
            end: Point::new(40.0, 0.0),
            stops: vec![
                GradientStop::new(0.0, Color::BLACK),
                GradientStop::new(1.0, Color::WHITE),
            ],
        }));
        let grey = |frame: &Frame, x: u32| i32::from(frame.premultiplied_pixel(x, 0).unwrap()[0]);

        let double = renderer
            .render(&gradient, &RenderOptions::new().with_scale(2.0))
            .unwrap();
        assert_eq!((double.width(), double.height()), (80, 40));
        assert!(
            (grey(&double, 40) - 128).abs() <= 3,
            "{}",
            grey(&double, 40)
        );

        // The right half only: it starts at mid grey.
        let right = RenderOptions::new().with_region(Rect::new(20.0, 0.0, 20.0, 20.0));
        let frame = renderer.render(&gradient, &right).unwrap();
        assert_eq!((frame.width(), frame.height()), (20, 20));
        assert!((grey(&frame, 0) - 130).abs() <= 3, "{}", grey(&frame, 0));
        assert!(grey(&frame, 19) > 245);

        // A region that reaches past the artboard: nothing is drawn there.
        let past = RenderOptions::new().with_region(Rect::new(30.0, 10.0, 20.0, 20.0));
        let frame = renderer.render(&gradient, &past).unwrap();
        assert_eq!(frame.premultiplied_pixel(9, 9).unwrap()[3], 255);
        assert_eq!(frame.premultiplied_pixel(10, 9), Some([0, 0, 0, 0]));
        assert_eq!(frame.premultiplied_pixel(9, 10), Some([0, 0, 0, 0]));
    }

    #[test]
    fn frames_of_the_same_size_are_reused() {
        let renderer = renderer();
        let red = scene(Some(ScenePaint::solid(Color::rgb(255, 0, 0))));
        let mut frame = renderer.render(&red, &RenderOptions::new()).unwrap();
        let memory = frame.premultiplied().as_ptr();

        // Drawn again from clear, in the same memory.
        let mut half = scene(None);
        half.nodes = vec![square("s", 0.0, 0.0, 10.0, Color::BLACK)];
        renderer
            .render_to(&half, &RenderOptions::new(), &mut frame)
            .unwrap();
        assert_eq!(frame.premultiplied().as_ptr(), memory);
        assert_eq!(frame.premultiplied_pixel(5, 5), Some([0, 0, 0, 255]));
        assert_eq!(frame.premultiplied_pixel(30, 5), Some([0, 0, 0, 0]));
        assert_eq!(
            frame,
            renderer.render(&half, &RenderOptions::new()).unwrap()
        );

        // Another size gets new memory.
        renderer
            .render_to(&half, &RenderOptions::new().with_scale(2.0), &mut frame)
            .unwrap();
        assert_eq!((frame.width(), frame.height()), (80, 40));
        let error = renderer.render_to(&half, &RenderOptions::new().with_scale(0.0), &mut frame);
        assert!(error.is_err());
        assert_eq!(frame.width(), 80, "left as it was");
    }

    #[test]
    fn a_callers_canvas_is_drawn_on_and_left_as_it_was() {
        let renderer = renderer();
        let filled = scene(Some(ScenePaint::solid(Color::rgb(255, 0, 0))));
        let mut surface = sk::surfaces::raster_n32_premul((100, 100)).unwrap();
        let canvas = surface.canvas();
        canvas.translate((10.0, 30.0));
        let before = (canvas.save_count(), canvas.local_to_device_as_3x3());
        assert!(renderer.render_into(&filled, canvas).is_empty());
        assert_eq!(
            (canvas.save_count(), canvas.local_to_device_as_3x3()),
            before
        );
        let image = surface.image_snapshot();
        let pixmap = image.peek_pixels().unwrap();
        assert_eq!(pixmap.get_color((49, 49)), sk::Color::RED);
        assert_eq!(pixmap.get_color((50, 49)), sk::Color::TRANSPARENT);
        assert_eq!(pixmap.get_color((9, 30)), sk::Color::TRANSPARENT);
    }

    fn node(id: &str, transform: Affine, width: f64, height: f64, content: Content) -> SceneNode {
        SceneNode {
            id: NodeId::from(id),
            transform,
            width,
            height,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            blur: 0.0,
            shadow: None,
            clip: None,
            mask: None,
            content,
        }
    }

    fn square(id: &str, x: f64, y: f64, size: f64, color: Color) -> SceneNode {
        let content = Content::Rect {
            radii: [0.0; 4],
            fill: Some(ScenePaint::solid(color)),
            stroke: None,
        };
        node(id, Affine::translate(x, y), size, size, content)
    }

    #[test]
    fn nodes_are_drawn_back_to_front_through_nested_transforms() {
        let renderer = renderer();
        let red = Color::rgb(255, 0, 0);
        let green = Color::rgb(0, 255, 0);
        // A group at (10, 0), scaled 2, holding a 5 by 5 square at (2, 2):
        // the square lands at (14, 4) and is 10 by 10.
        let group = node(
            "group",
            Affine::translate(10.0, 0.0) * Affine::scale(2.0, 2.0),
            10.0,
            10.0,
            Content::Group {
                children: vec![square("inner", 2.0, 2.0, 5.0, green)],
            },
        );
        let mut scene = scene(None);
        scene.nodes = vec![square("back", 0.0, 0.0, 20.0, red), group];
        let frame = renderer.render(&scene, &RenderOptions::new()).unwrap();
        let pixel = |x, y| frame.premultiplied_pixel(x, y).unwrap();
        assert_eq!(pixel(13, 4), [255, 0, 0, 255], "left of the inner square");
        assert_eq!(pixel(14, 4), [0, 255, 0, 255], "the later node is in front");
        assert_eq!(pixel(23, 13), [0, 255, 0, 255]);
        assert_eq!(pixel(24, 13), [0, 0, 0, 0]);
        assert_eq!(pixel(14, 14), [255, 0, 0, 255]);

        // A quarter turn about the centre of a 20 by 10 box at (10, 5)
        // stands it upright: 10 wide, 20 tall, around the same centre.
        let mut turned = self::scene(None);
        let mut bar = square("bar", 0.0, 0.0, 0.0, red);
        (bar.width, bar.height) = (20.0, 10.0);
        bar.transform = Affine::layer(10.0, 5.0, 20.0, 10.0, 90.0, 1.0);
        turned.nodes = vec![bar];
        let frame = renderer.render(&turned, &RenderOptions::new()).unwrap();
        let alpha = |x, y| frame.premultiplied_pixel(x, y).unwrap()[3];
        assert_eq!((alpha(20, 1), alpha(20, 18)), (255, 255));
        assert_eq!((alpha(12, 10), alpha(27, 10)), (0, 0));
    }

    #[test]
    fn opacity_fades_a_node_as_a_whole() {
        let renderer = renderer();
        let black = Color::BLACK;
        // Two overlapping squares in a half-transparent group: the overlap
        // is no darker than the rest.
        let mut group = node(
            "group",
            Affine::IDENTITY,
            40.0,
            20.0,
            Content::Group {
                children: vec![
                    square("a", 0.0, 0.0, 12.0, black),
                    square("b", 6.0, 6.0, 12.0, black),
                ],
            },
        );
        group.opacity = 0.5;
        let mut scene = scene(None);
        scene.nodes = vec![group];
        let frame = renderer.render(&scene, &RenderOptions::new()).unwrap();
        let alpha = |x, y| i32::from(frame.premultiplied_pixel(x, y).unwrap()[3]);
        assert!((alpha(2, 2) - 128).abs() <= 1, "{}", alpha(2, 2));
        assert_eq!(alpha(2, 2), alpha(9, 9), "the overlap");
        assert_eq!(alpha(2, 2), alpha(15, 15));

        // A node that cannot be seen or placed is skipped without fuss.
        let mut hidden = square("hidden", 0.0, 0.0, 10.0, black);
        hidden.opacity = 0.0;
        let mut flattened = square("flat", 0.0, 0.0, 10.0, black);
        flattened.transform = Affine::scale(0.0, 1.0);
        let mut broken = square("broken", 0.0, 0.0, 10.0, black);
        broken.transform = Affine([f64::NAN; 6]);
        scene.nodes = vec![hidden, flattened, broken];
        let frame = renderer.render(&scene, &RenderOptions::new()).unwrap();
        assert!(frame.premultiplied().iter().all(|byte| *byte == 0));
    }

    /// Renders `nodes` on a transparent 40 by 20 artboard.
    fn draw(nodes: Vec<SceneNode>) -> Frame {
        let mut scene = scene(None);
        scene.nodes = nodes;
        renderer().render(&scene, &RenderOptions::new()).unwrap()
    }

    fn group(children: Vec<SceneNode>) -> SceneNode {
        node(
            "group",
            Affine::IDENTITY,
            40.0,
            20.0,
            Content::Group { children },
        )
    }

    #[test]
    fn clips_cut_with_rounded_corners() {
        let mut clipped = group(vec![square("s", 0.0, 0.0, 20.0, Color::BLACK)]);
        clipped.clip = Some(Clip {
            rect: Rect::new(0.0, 0.0, 10.0, 10.0),
            radii: [5.0; 4],
        });
        let frame = draw(vec![clipped]);
        let alpha = |x, y| frame.premultiplied_pixel(x, y).unwrap()[3];
        assert_eq!(alpha(0, 0), 0, "the rounded corner");
        assert_eq!(alpha(5, 5), 255);
        assert_eq!(alpha(8, 5), 255);
        assert!((1..255).contains(&alpha(9, 5)), "the smoothed edge");
        assert_eq!(alpha(10, 5), 0, "past the clip");
        assert_eq!(alpha(15, 15), 0);
    }

    #[test]
    fn blend_modes_mix_with_what_is_behind() {
        let mix = |mode| {
            let mut red = square("red", 0.0, 0.0, 10.0, Color::rgb(255, 0, 0));
            red.blend_mode = mode;
            let frame = draw(vec![
                square("blue", 0.0, 0.0, 10.0, Color::rgb(0, 0, 255)),
                red,
            ]);
            frame.premultiplied_pixel(5, 5).unwrap()
        };
        assert_eq!(mix(BlendMode::Normal), [255, 0, 0, 255]);
        assert_eq!(mix(BlendMode::Multiply), [0, 0, 0, 255]);
        assert_eq!(mix(BlendMode::Screen), [255, 0, 255, 255]);
        assert_eq!(mix(BlendMode::Lighten), [255, 0, 255, 255]);
        assert_eq!(mix(BlendMode::Difference), [255, 0, 255, 255]);
        // Over nothing, a blend mode draws the layer as it is.
        let mut alone = square("alone", 0.0, 0.0, 10.0, Color::rgb(255, 0, 0));
        alone.blend_mode = BlendMode::Multiply;
        assert_eq!(
            draw(vec![alone]).premultiplied_pixel(5, 5),
            Some([255, 0, 0, 255])
        );
    }

    #[test]
    fn shadows_sit_behind_and_blur_softens() {
        let mut shadowed = square("s", 2.0, 2.0, 8.0, Color::WHITE);
        shadowed.shadow = Some(Shadow {
            color: Color::rgba(0, 0, 0, 128),
            x: 20.0,
            y: 0.0,
            blur: 0.0,
        });
        let frame = draw(vec![shadowed.clone()]);
        assert_eq!(frame.premultiplied_pixel(5, 5), Some([255, 255, 255, 255]));
        assert_eq!(frame.premultiplied_pixel(25, 5), Some([0, 0, 0, 128]));
        assert_eq!(frame.premultiplied_pixel(35, 5), Some([0, 0, 0, 0]));

        // Blur spreads the edge and lowers the middle of a small shape.
        let mut blurred = square("b", 10.0, 2.0, 16.0, Color::BLACK);
        blurred.blur = 8.0;
        let frame = draw(vec![blurred]);
        let alpha = |x, y| frame.premultiplied_pixel(x, y).unwrap()[3];
        assert!(alpha(8, 10) > 20, "outside the edge: {}", alpha(8, 10));
        assert!((100..250).contains(&alpha(10, 10)), "{}", alpha(10, 10));
        assert!(alpha(18, 10) > alpha(11, 10));

        // The blur covers the shadow too; the clip cuts both.
        let mut both = shadowed;
        both.blur = 4.0;
        let frame = draw(vec![both.clone()]);
        let alpha = |frame: &Frame, x| frame.premultiplied_pixel(x, 5).unwrap()[3];
        assert!(
            (1..128).contains(&alpha(&frame, 21)),
            "{}",
            alpha(&frame, 21)
        );
        both.clip = Some(Clip {
            rect: Rect::new(0.0, 0.0, 12.0, 12.0),
            radii: [0.0; 4],
        });
        let frame = draw(vec![both]);
        assert_eq!(alpha(&frame, 21), 0);
        assert!(alpha(&frame, 11) > 0, "the blurred edge inside the clip");
    }

    #[test]
    fn masks_keep_what_their_alpha_covers() {
        let content = square("content", 0.0, 0.0, 20.0, Color::rgb(0, 255, 0));
        let mut masked = group(vec![content]);
        let mut mask = square("mask", 0.0, 0.0, 10.0, Color::BLACK);
        (mask.width, mask.height) = (10.0, 20.0);
        mask.opacity = 0.5;
        masked.mask = Some(Box::new(mask));
        let frame = draw(vec![
            square("behind", 30.0, 0.0, 10.0, Color::rgb(0, 0, 255)),
            masked,
        ]);
        let pixel = |x, y| frame.premultiplied_pixel(x, y).unwrap();
        let half = pixel(5, 5);
        assert!(
            half[1].abs_diff(128) <= 1 && half[3].abs_diff(128) <= 1,
            "{half:?}"
        );
        assert_eq!(pixel(15, 5), [0, 0, 0, 0], "outside the mask");
        assert_eq!(pixel(35, 5), [0, 0, 255, 255], "the mask cuts nothing else");
    }

    /// An image layer at the origin showing a 20 by 10 picture: red on the
    /// left half, blue on the right.
    fn picture(width: f64, height: f64, fit: ImageFit, radii: [f64; 4]) -> SceneNode {
        let png = crate::assets::tests::png(20, 10, |x, _| {
            if x < 10 {
                [255, 0, 0, 255]
            } else {
                [0, 0, 255, 255]
            }
        });
        let content = Content::Image {
            source: moshionly_core::io::data_uri("image/png", &png),
            fit,
            radii,
            stroke: None,
        };
        node("photo", Affine::IDENTITY, width, height, content)
    }

    #[test]
    fn images_fill_their_box_by_fit() {
        let pixel = |frame: &Frame, x, y| frame.premultiplied_pixel(x, y).unwrap();
        let red = [255, 0, 0, 255];
        let blue = [0, 0, 255, 255];

        let stretched = draw(vec![picture(40.0, 20.0, ImageFit::Stretch, [0.0; 4])]);
        assert!(
            stretched.warnings().is_empty(),
            "{:?}",
            stretched.warnings()
        );
        assert_eq!(pixel(&stretched, 5, 10), red);
        assert_eq!(pixel(&stretched, 35, 10), blue);

        // A square box: cover crops the sides, contain leaves bands above
        // and below.
        let cover = draw(vec![picture(20.0, 20.0, ImageFit::Cover, [0.0; 4])]);
        assert_eq!(pixel(&cover, 2, 1), red);
        assert_eq!(pixel(&cover, 17, 18), blue);
        let contain = draw(vec![picture(20.0, 20.0, ImageFit::Contain, [0.0; 4])]);
        assert_eq!(pixel(&contain, 2, 2), [0, 0, 0, 0]);
        assert_eq!(pixel(&contain, 2, 10), red);
        assert_eq!(pixel(&contain, 17, 17), [0, 0, 0, 0]);

        // Rounded corners cut the picture.
        let rounded = draw(vec![picture(20.0, 20.0, ImageFit::Cover, [8.0; 4])]);
        assert_eq!(pixel(&rounded, 0, 0), [0, 0, 0, 0]);
        assert_eq!(pixel(&rounded, 8, 8), red);
    }

    #[test]
    fn a_missing_image_draws_a_placeholder_and_says_so() {
        let mut missing = picture(40.0, 20.0, ImageFit::Cover, [0.0; 4]);
        missing.content = Content::Image {
            source: "/no/such/photo.png".into(),
            fit: ImageFit::Cover,
            radii: [0.0; 4],
            stroke: None,
        };
        let frame = draw(vec![missing]);
        assert_eq!(frame.premultiplied_pixel(30, 4).unwrap()[3], 255, "drawn");
        match frame.warnings() {
            [
                RenderWarning::MissingImage {
                    node,
                    source,
                    reason,
                },
            ] => {
                assert_eq!(node, &NodeId::from("photo"));
                assert_eq!(source, "/no/such/photo.png");
                assert!(reason.contains("no file"), "{reason}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_renderer_can_go_to_another_thread() {
        fn sendable<T: Send + Sync>() {}
        sendable::<Renderer>();
        sendable::<Frame>();
        let renderer = renderer();
        let scene = scene(Some(ScenePaint::solid(Color::WHITE)));
        let frame = std::thread::spawn(move || renderer.render(&scene, &RenderOptions::new()))
            .join()
            .unwrap()
            .unwrap();
        assert!(frame.is_opaque());
    }
}
