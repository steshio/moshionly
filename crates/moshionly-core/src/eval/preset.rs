//! Presets: one node in the document, a few property changes at evaluation.
//!
//! A preset such as `slide_in` is stored with its own small settings and is
//! expanded here into what it does to a layer: where it moves from, how it
//! fades, how it scales. The expansion never touches the layer's own
//! properties or its property animations. It goes into a second set of
//! channels, the layer's *pose*, which is laid on top:
//!
//! | Pose channel | Neutral | Combines with the layer's own value by |
//! |---|---|---|
//! | opacity | 1 | multiplying |
//! | scale | 1 | multiplying |
//! | offset | 0, 0 | adding (in the parent's coordinates, like `move`) |
//! | rotation | 0 | adding |
//! | blur | 0 | adding |
//! | reveal | the whole box | clipping |
//!
//! That is why a `slide_in` and a separate `move` both take effect, and why
//! a `fade_in` on a layer at 50% opacity ends at 50%.
//!
//! Within one pose channel, presets follow the replacing rule: the most
//! recently started preset decides. An exit (any `*_out`) starts from
//! wherever the pose is at that moment, so it takes over smoothly from an
//! entrance that has not finished. An entrance (any `*_in`) starts from its
//! own start value, and returns to neutral any channel it does not use that
//! an exit left elsewhere, so a layer that slid out and later fades back in
//! comes back in place.

use crate::geometry::Point;
use crate::model::{Direction, NodeData};

/// The whole box, as `[left, top, right, bottom]` fractions of it.
pub(crate) const WHOLE_BOX: [f64; 4] = [0.0, 0.0, 1.0, 1.0];

/// What one preset does to a layer's pose: for every channel it uses, the
/// value it starts from and the value it ends on. For an exit the start
/// value is the neutral one and only says what the exit departs from when
/// nothing came before.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct Expansion {
    pub opacity: Option<(f64, f64)>,
    pub scale: Option<(f64, f64)>,
    pub offset: Option<(Point, Point)>,
    pub rotation: Option<(f64, f64)>,
    pub blur: Option<(f64, f64)>,
    /// The part of the box that shows, as fractions of it.
    pub reveal: Option<([f64; 4], [f64; 4])>,
    /// Whether the preset brings the layer in, which resets the channels it
    /// does not use.
    pub entrance: bool,
}

const FADE_IN: Option<(f64, f64)> = Some((0.0, 1.0));
const FADE_OUT: Option<(f64, f64)> = Some((1.0, 0.0));

/// The sliver of the box that is left when a wipe travelling in `direction`
/// has not uncovered anything yet: the edge it starts from.
fn leading_edge(direction: Direction) -> [f64; 4] {
    match direction {
        Direction::Right => [0.0, 0.0, 0.0, 1.0],
        Direction::Left => [1.0, 0.0, 1.0, 1.0],
        Direction::Down => [0.0, 0.0, 1.0, 0.0],
        Direction::Up => [0.0, 1.0, 1.0, 1.0],
    }
}

/// The sliver of the box that is left when a wipe travelling in `direction`
/// has covered everything: the edge it arrives at.
fn trailing_edge(direction: Direction) -> [f64; 4] {
    match direction {
        Direction::Right => leading_edge(Direction::Left),
        Direction::Left => leading_edge(Direction::Right),
        Direction::Down => leading_edge(Direction::Up),
        Direction::Up => leading_edge(Direction::Down),
    }
}

/// Expands a preset. `None` for nodes that are not presets.
pub(crate) fn expand(preset: &NodeData) -> Option<Expansion> {
    let fade = |on: bool, values: Option<(f64, f64)>| if on { values } else { None };
    let travel = |direction: Direction, distance: f64| {
        let (x, y) = direction.vector();
        Point::new(x * distance, y * distance)
    };
    let mut expansion = match preset {
        NodeData::FadeIn(_) => Expansion {
            opacity: FADE_IN,
            ..Expansion::default()
        },
        NodeData::FadeOut(_) => Expansion {
            opacity: FADE_OUT,
            ..Expansion::default()
        },
        // Sliding in travels in the direction and arrives at rest, so it
        // starts a distance away on the opposite side.
        NodeData::SlideIn(slide) => {
            let way = travel(slide.direction, slide.distance);
            Expansion {
                offset: Some((Point::new(-way.x, -way.y), Point::ZERO)),
                opacity: fade(slide.fade, FADE_IN),
                ..Expansion::default()
            }
        }
        NodeData::SlideOut(slide) => Expansion {
            offset: Some((Point::ZERO, travel(slide.direction, slide.distance))),
            opacity: fade(slide.fade, FADE_OUT),
            ..Expansion::default()
        },
        NodeData::GrowIn(grow) => Expansion {
            scale: Some((grow.scale, 1.0)),
            opacity: fade(grow.fade, FADE_IN),
            ..Expansion::default()
        },
        NodeData::GrowOut(grow) => Expansion {
            scale: Some((1.0, grow.scale)),
            opacity: fade(grow.fade, FADE_OUT),
            ..Expansion::default()
        },
        NodeData::ShrinkIn(shrink) => Expansion {
            scale: Some((shrink.scale, 1.0)),
            opacity: fade(shrink.fade, FADE_IN),
            ..Expansion::default()
        },
        NodeData::ShrinkOut(shrink) => Expansion {
            scale: Some((1.0, shrink.scale)),
            opacity: fade(shrink.fade, FADE_OUT),
            ..Expansion::default()
        },
        NodeData::SpinIn(spin) => Expansion {
            scale: Some((0.0, 1.0)),
            rotation: Some((-spin.angle, 0.0)),
            opacity: fade(spin.fade, FADE_IN),
            ..Expansion::default()
        },
        NodeData::SpinOut(spin) => Expansion {
            scale: Some((1.0, 0.0)),
            rotation: Some((0.0, spin.angle)),
            opacity: fade(spin.fade, FADE_OUT),
            ..Expansion::default()
        },
        NodeData::BlurIn(blur) => Expansion {
            blur: Some((blur.blur, 0.0)),
            opacity: FADE_IN,
            ..Expansion::default()
        },
        NodeData::BlurOut(blur) => Expansion {
            blur: Some((0.0, blur.blur)),
            opacity: FADE_OUT,
            ..Expansion::default()
        },
        NodeData::RevealIn(reveal) => Expansion {
            reveal: Some((leading_edge(reveal.direction), WHOLE_BOX)),
            ..Expansion::default()
        },
        NodeData::RevealOut(reveal) => Expansion {
            reveal: Some((WHOLE_BOX, trailing_edge(reveal.direction))),
            ..Expansion::default()
        },
        _ => return None,
    };
    expansion.entrance = preset.node_type().is_entrance();
    Some(expansion)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::default_animation;
    use crate::id::NodeId;
    use crate::model::{Family, NodeType};

    fn expanded(ty: NodeType) -> Expansion {
        expand(&default_animation(ty, &NodeId::from("x"), 0.0).unwrap()).unwrap()
    }

    #[test]
    fn every_preset_expands_and_nothing_else_does() {
        for ty in NodeType::ALL {
            let Some(data) = default_animation(*ty, &NodeId::from("x"), 0.0) else {
                continue;
            };
            let is_preset = ty.is_entrance() || ty.is_exit();
            assert_eq!(expand(&data).is_some(), is_preset, "{ty}");
            if let Some(expansion) = expand(&data) {
                assert_eq!(expansion.entrance, ty.is_entrance(), "{ty}");
            }
        }
        assert_eq!(
            NodeType::ALL
                .iter()
                .filter(|ty| ty.family() == Family::Animation && (ty.is_entrance() || ty.is_exit()))
                .count(),
            14
        );
    }

    #[test]
    fn entrances_end_neutral_and_exits_start_neutral() {
        for ty in NodeType::ALL {
            if !(ty.is_entrance() || ty.is_exit()) {
                continue;
            }
            let expansion = expanded(*ty);
            let neutral = |pair: (f64, f64), value: f64| {
                if ty.is_entrance() {
                    pair.1 == value
                } else {
                    pair.0 == value
                }
            };
            assert!(
                expansion.opacity.is_none_or(|pair| neutral(pair, 1.0)),
                "{ty}"
            );
            assert!(
                expansion.scale.is_none_or(|pair| neutral(pair, 1.0)),
                "{ty}"
            );
            assert!(
                expansion.rotation.is_none_or(|pair| neutral(pair, 0.0)),
                "{ty}"
            );
            assert!(expansion.blur.is_none_or(|pair| neutral(pair, 0.0)), "{ty}");
            if let Some((from, to)) = expansion.offset {
                let rest = if ty.is_entrance() { to } else { from };
                assert_eq!(rest, Point::ZERO, "{ty}");
            }
            if let Some((from, to)) = expansion.reveal {
                assert_eq!(if ty.is_entrance() { to } else { from }, WHOLE_BOX, "{ty}");
            }
        }
    }

    #[test]
    fn a_slide_in_starts_on_the_side_it_comes_from() {
        // The default direction is up: it starts below, at positive y.
        let slide = expanded(NodeType::SlideIn);
        assert_eq!(slide.offset, Some((Point::new(0.0, 100.0), Point::ZERO)));
        assert_eq!(slide.opacity, Some((0.0, 1.0)));
        let out = expanded(NodeType::SlideOut);
        assert_eq!(out.offset, Some((Point::ZERO, Point::new(0.0, -100.0))));
    }

    #[test]
    fn wipes_uncover_from_one_edge_and_cover_toward_the_other() {
        // Travelling right, a reveal starts as a sliver on the left edge and
        // a cover ends as a sliver on the right edge.
        assert_eq!(leading_edge(Direction::Right), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(trailing_edge(Direction::Right), [1.0, 0.0, 1.0, 1.0]);
        assert_eq!(leading_edge(Direction::Down), [0.0, 0.0, 1.0, 0.0]);
        assert_eq!(trailing_edge(Direction::Down), [0.0, 1.0, 1.0, 1.0]);
        assert_eq!(leading_edge(Direction::Up), trailing_edge(Direction::Down));
        assert_eq!(
            leading_edge(Direction::Left),
            trailing_edge(Direction::Right)
        );
        let reveal = expanded(NodeType::RevealIn);
        assert_eq!(reveal.reveal, Some(([0.0, 0.0, 0.0, 1.0], WHOLE_BOX)));
        assert!(reveal.opacity.is_none());
    }
}
