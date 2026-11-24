//! Reading the paths out of the SVG that Skia's SVG canvas writes.
//!
//! Text is turned into outlines by drawing it with the renderer onto an
//! SVG canvas that converts text to paths. What comes back is a small SVG
//! document of `<path>` elements, each with path data (`d`) and perhaps a
//! `transform`, inside groups that may have transforms of their own. This
//! reads those paths into contours; it is not a general SVG reader, and
//! anything else in the document (clip paths, definitions) is skipped.

use super::shapes::Contour;

/// A 2D affine transform `[a, b, c, d, e, f]`, as SVG's `matrix(...)`.
type Matrix = [f64; 6];

const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

fn multiply(m: Matrix, n: Matrix) -> Matrix {
    [
        m[0] * n[0] + m[2] * n[1],
        m[1] * n[0] + m[3] * n[1],
        m[0] * n[2] + m[2] * n[3],
        m[1] * n[2] + m[3] * n[3],
        m[0] * n[4] + m[2] * n[5] + m[4],
        m[1] * n[4] + m[3] * n[5] + m[5],
    ]
}

fn apply(m: Matrix, [x, y]: [f64; 2]) -> [f64; 2] {
    [m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]]
}

/// The contours of every `<path>` in `svg` that is not inside a
/// `<clipPath>` or `<defs>`, with their transforms applied.
pub(crate) fn paths(svg: &str) -> Vec<Contour> {
    let mut contours = Vec::new();
    let mut transforms = vec![IDENTITY];
    let mut hidden = 0usize;
    let mut rest = svg;
    while let Some(start) = rest.find('<') {
        rest = &rest[start + 1..];
        let end = rest.find('>').unwrap_or(rest.len());
        let tag = &rest[..end];
        rest = &rest[(end + 1).min(rest.len())..];
        let self_closing = tag.ends_with('/');
        let name: String = tag
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        let closing = tag.starts_with('/');
        match (name.as_str(), closing) {
            ("clipPath" | "defs" | "mask" | "pattern", false) if !self_closing => hidden += 1,
            ("clipPath" | "defs" | "mask" | "pattern", true) => hidden = hidden.saturating_sub(1),
            ("g", false) if !self_closing => {
                let parent = *transforms.last().unwrap_or(&IDENTITY);
                let own = attribute(tag, "transform").map_or(IDENTITY, transform);
                transforms.push(multiply(parent, own));
            }
            ("g", true) => {
                if transforms.len() > 1 {
                    transforms.pop();
                }
            }
            ("path", false) if hidden == 0 => {
                let parent = *transforms.last().unwrap_or(&IDENTITY);
                let own = attribute(tag, "transform").map_or(IDENTITY, transform);
                if let Some(data) = attribute(tag, "d") {
                    let matrix = multiply(parent, own);
                    contours.extend(path_data(data).into_iter().map(|c| transformed(c, matrix)));
                }
            }
            _ => {}
        }
    }
    contours
}

/// The value of attribute `name` in a tag's text.
fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let mut search = tag;
    loop {
        let at = search.find(name)?;
        let before = search[..at].chars().last();
        let after = &search[at + name.len()..];
        if before.is_some_and(char::is_whitespace) && after.trim_start().starts_with('=') {
            let value = after.trim_start()[1..].trim_start();
            let quote = value.chars().next()?;
            let value = &value[1..];
            return value.find(quote).map(|end| &value[..end]);
        }
        search = after;
    }
}

/// An SVG `transform` attribute: `matrix`, `translate` and `scale`, one
/// after another.
fn transform(text: &str) -> Matrix {
    let mut matrix = IDENTITY;
    let mut rest = text;
    while let Some(open) = rest.find('(') {
        let name = rest[..open].trim();
        let close = rest[open..].find(')').map_or(rest.len(), |at| open + at);
        let numbers = numbers(&rest[open + 1..close]);
        let own = match (name, numbers.as_slice()) {
            ("matrix", [a, b, c, d, e, f]) => [*a, *b, *c, *d, *e, *f],
            ("translate", [x]) => [1.0, 0.0, 0.0, 1.0, *x, 0.0],
            ("translate", [x, y]) => [1.0, 0.0, 0.0, 1.0, *x, *y],
            ("scale", [s]) => [*s, 0.0, 0.0, *s, 0.0, 0.0],
            ("scale", [x, y]) => [*x, 0.0, 0.0, *y, 0.0, 0.0],
            _ => IDENTITY,
        };
        matrix = multiply(matrix, own);
        rest = &rest[(close + 1).min(rest.len())..];
    }
    matrix
}

/// The numbers in a list, separated by spaces or commas.
fn numbers(text: &str) -> Vec<f64> {
    text.split(|c: char| c.is_whitespace() || c == ',')
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse().ok())
        .collect()
}

/// A contour with `matrix` applied: points moved, handles turned.
fn transformed(mut contour: Contour, matrix: Matrix) -> Contour {
    if matrix == IDENTITY {
        return contour;
    }
    let linear = [matrix[0], matrix[1], matrix[2], matrix[3], 0.0, 0.0];
    for vertex in &mut contour.vertices {
        *vertex = apply(matrix, *vertex);
    }
    for handle in contour.ins.iter_mut().chain(&mut contour.outs) {
        *handle = apply(linear, *handle);
    }
    contour
}

/// Splits path data into commands and their numbers.
fn tokens(data: &str) -> Vec<(char, Vec<f64>)> {
    let mut commands: Vec<(char, Vec<f64>)> = Vec::new();
    let mut number = String::new();
    let flush = |number: &mut String, commands: &mut Vec<(char, Vec<f64>)>| {
        if let (Ok(value), Some(last)) = (number.parse::<f64>(), commands.last_mut()) {
            last.1.push(value);
        }
        number.clear();
    };
    for c in data.chars() {
        match c {
            'e' | 'E' => number.push(c),
            c if c.is_ascii_alphabetic() => {
                flush(&mut number, &mut commands);
                commands.push((c, Vec::new()));
            }
            '-' | '+' if !number.ends_with(['e', 'E']) => {
                flush(&mut number, &mut commands);
                number.push(c);
            }
            '.' if number.contains('.') && !number.contains(['e', 'E']) => {
                // "1.5.5" is two numbers.
                flush(&mut number, &mut commands);
                number.push(c);
            }
            c if c.is_ascii_digit() || c == '.' || c == '-' || c == '+' => number.push(c),
            _ => flush(&mut number, &mut commands),
        }
    }
    flush(&mut number, &mut commands);
    commands
}

/// The contours of SVG path data. Quadratic curves become the cubic curves
/// they are; arcs, which Skia does not write, become straight lines.
pub(crate) fn path_data(data: &str) -> Vec<Contour> {
    let mut contours: Vec<Contour> = Vec::new();
    let mut current: Option<Contour> = None;
    let (mut point, mut start) = ([0.0, 0.0], [0.0, 0.0]);
    // The last control point, for smooth curves.
    let mut last_control: Option<(char, [f64; 2])> = None;
    let finish = |current: &mut Option<Contour>, contours: &mut Vec<Contour>| {
        if let Some(contour) = current.take()
            && contour.vertices.len() > 1
        {
            contours.push(contour);
        }
    };
    for (command, values) in tokens(data) {
        let relative = command.is_ascii_lowercase();
        let upper = command.to_ascii_uppercase();
        let at = |base: [f64; 2], x: f64, y: f64| {
            if relative {
                [base[0] + x, base[1] + y]
            } else {
                [x, y]
            }
        };
        let arity = match upper {
            'M' | 'L' | 'T' => 2,
            'H' | 'V' => 1,
            'C' => 6,
            'S' | 'Q' => 4,
            'A' => 7,
            _ => 0,
        };
        if upper == 'Z' {
            if let Some(contour) = current.as_mut() {
                contour.close();
            }
            finish(&mut current, &mut contours);
            point = start;
            last_control = None;
            continue;
        }
        if arity == 0 {
            continue;
        }
        for (index, chunk) in values.chunks_exact(arity).enumerate() {
            // Numbers after a move are lines.
            let upper = if upper == 'M' && index > 0 {
                'L'
            } else {
                upper
            };
            match upper {
                'M' => {
                    finish(&mut current, &mut contours);
                    point = at(point, chunk[0], chunk[1]);
                    start = point;
                    current = Some(Contour::starting_at(point));
                    last_control = None;
                }
                _ => {
                    let contour = current.get_or_insert_with(|| Contour::starting_at(point));
                    match upper {
                        'L' | 'A' => {
                            let (x, y) = if upper == 'A' {
                                (chunk[5], chunk[6])
                            } else {
                                (chunk[0], chunk[1])
                            };
                            point = at(point, x, y);
                            contour.line_to(point);
                            last_control = None;
                        }
                        'H' => {
                            point = [
                                if relative {
                                    point[0] + chunk[0]
                                } else {
                                    chunk[0]
                                },
                                point[1],
                            ];
                            contour.line_to(point);
                            last_control = None;
                        }
                        'V' => {
                            point = [
                                point[0],
                                if relative {
                                    point[1] + chunk[0]
                                } else {
                                    chunk[0]
                                },
                            ];
                            contour.line_to(point);
                            last_control = None;
                        }
                        'C' | 'S' => {
                            let (c1, c2, end) = if upper == 'C' {
                                (
                                    at(point, chunk[0], chunk[1]),
                                    at(point, chunk[2], chunk[3]),
                                    at(point, chunk[4], chunk[5]),
                                )
                            } else {
                                let c1 = match last_control {
                                    Some(('C', c)) => {
                                        [2.0 * point[0] - c[0], 2.0 * point[1] - c[1]]
                                    }
                                    _ => point,
                                };
                                (
                                    c1,
                                    at(point, chunk[0], chunk[1]),
                                    at(point, chunk[2], chunk[3]),
                                )
                            };
                            contour.cubic_to(c1, c2, end);
                            last_control = Some(('C', c2));
                            point = end;
                        }
                        'Q' | 'T' => {
                            let (control, end) = if upper == 'Q' {
                                (at(point, chunk[0], chunk[1]), at(point, chunk[2], chunk[3]))
                            } else {
                                let control = match last_control {
                                    Some(('Q', c)) => {
                                        [2.0 * point[0] - c[0], 2.0 * point[1] - c[1]]
                                    }
                                    _ => point,
                                };
                                (control, at(point, chunk[0], chunk[1]))
                            };
                            let third = |from: [f64; 2]| {
                                [
                                    from[0] + (control[0] - from[0]) * 2.0 / 3.0,
                                    from[1] + (control[1] - from[1]) * 2.0 / 3.0,
                                ]
                            };
                            contour.cubic_to(third(point), third(end), end);
                            last_control = Some(('Q', control));
                            point = end;
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    finish(&mut current, &mut contours);
    contours
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_read_with_their_transforms() {
        let svg = r#"<svg><clipPath id="c"><path d="M0 0L9 9Z"/></clipPath><g transform="translate(10 20)"><path transform="matrix(2 0 0 2 1 1)" d="M0 0L5 0L5 5Z"/></g></svg>"#;
        let contours = paths(svg);
        assert_eq!(contours.len(), 1, "the clip path is skipped");
        assert!(contours[0].closed);
        assert_eq!(
            contours[0].vertices,
            [[11.0, 21.0], [21.0, 21.0], [21.0, 31.0]]
        );
    }

    #[test]
    fn curves_become_cubics() {
        let contours = path_data("M0 0Q10 0 10 10L0 10Z M20 20 C 21 21 22-22 23 23");
        assert_eq!(contours.len(), 2);
        let curve = &contours[0];
        assert_eq!(curve.vertices[..2], [[0.0, 0.0], [10.0, 10.0]]);
        // A quadratic's handles reach two thirds of the way to its control.
        let out = curve.outs[0];
        assert!((out[0] - 20.0 / 3.0).abs() < 1e-9 && out[1].abs() < 1e-9);
        assert_eq!(contours[1].vertices, [[20.0, 20.0], [23.0, 23.0]]);
        assert_eq!(contours[1].outs[0], [1.0, 1.0]);
        assert_eq!(contours[1].ins[1], [-1.0, -45.0]);
        assert!(!contours[1].closed);
    }

    #[test]
    fn numbers_in_every_spelling() {
        assert_eq!(
            tokens("m1.5.5-2e-1,3h4"),
            [('m', vec![1.5, 0.5, -0.2, 3.0]), ('h', vec![4.0])]
        );
    }
}
