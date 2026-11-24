//! The small amount of formatting the chat understands in an answer:
//! paragraphs, headings, bulleted and numbered lists, code blocks, and
//! inside a line `code`, **bold** and *italic*.
//!
//! [`parse`] turns text into [`Block`]s of [`Span`]s. It never fails: an
//! unclosed marker is shown as it is, and an unclosed code block runs to
//! the end, which is what half an answer looks like while it streams in.

/// A block of an answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// Lines of text. Single line breaks are kept.
    Paragraph(Vec<Span>),
    /// A heading line (`#`, `##` or `###`).
    Heading(Vec<Span>),
    /// One item of a bulleted list, indented `depth` levels.
    Bullet {
        /// How far the item is indented, from 0.
        depth: usize,
        /// The item's text.
        spans: Vec<Span>,
    },
    /// One item of a numbered list.
    Numbered {
        /// The number as written, for example `3.`.
        marker: String,
        /// The item's text.
        spans: Vec<Span>,
    },
    /// A fenced code block, shown as written.
    Code(String),
}

/// A run of text inside a block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Span {
    /// Ordinary text.
    Plain(String),
    /// Strong text.
    Bold(String),
    /// Emphasised text.
    Italic(String),
    /// Code, shown in the monospaced face.
    Code(String),
}

/// Splits `text` into blocks.
pub fn parse(text: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut paragraph: Vec<&str> = Vec::new();
    let mut lines = text.lines();
    let flush = |paragraph: &mut Vec<&str>, blocks: &mut Vec<Block>| {
        if !paragraph.is_empty() {
            blocks.push(Block::Paragraph(spans(&paragraph.join("\n"))));
            paragraph.clear();
        }
    };

    while let Some(line) = lines.next() {
        let trimmed = line.trim_start();
        if opens_code_block(trimmed) {
            flush(&mut paragraph, &mut blocks);
            let mut code = Vec::new();
            for line in lines.by_ref() {
                if line.trim_start().starts_with("```") {
                    break;
                }
                code.push(line);
            }
            blocks.push(Block::Code(code.join("\n")));
        } else if trimmed.is_empty() {
            flush(&mut paragraph, &mut blocks);
        } else if let Some(heading) = heading(trimmed) {
            flush(&mut paragraph, &mut blocks);
            blocks.push(Block::Heading(spans(heading)));
        } else if let Some(item) = bullet(trimmed) {
            flush(&mut paragraph, &mut blocks);
            let indent = line.len() - trimmed.len();
            blocks.push(Block::Bullet {
                depth: indent / 2,
                spans: spans(item),
            });
        } else if let Some((marker, item)) = numbered(trimmed) {
            flush(&mut paragraph, &mut blocks);
            blocks.push(Block::Numbered {
                marker: marker.to_owned(),
                spans: spans(item),
            });
        } else {
            paragraph.push(line.trim_end());
        }
    }
    flush(&mut paragraph, &mut blocks);
    blocks
}

/// True for a line that opens a code block: three or more backticks, then
/// at most the name of a language. A line such as ```` ```x``` and more ````
/// is ordinary text that starts with inline code.
fn opens_code_block(line: &str) -> bool {
    let info = line.trim_start_matches('`');
    line.len() - info.len() >= 3 && !info.contains('`')
}

/// The text of a heading line, without its `#`s.
fn heading(line: &str) -> Option<&str> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if !(1..=4).contains(&hashes) {
        return None;
    }
    line[hashes..]
        .strip_prefix(' ')
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

/// The text of a bulleted item, without its bullet.
fn bullet(line: &str) -> Option<&str> {
    ["- ", "* ", "• ", "+ "]
        .iter()
        .find_map(|bullet| line.strip_prefix(bullet))
}

/// The number and text of a numbered item, such as `2.` and the rest.
fn numbered(line: &str) -> Option<(&str, &str)> {
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 || digits > 3 {
        return None;
    }
    let rest = &line[digits..];
    let after = rest
        .strip_prefix(". ")
        .or_else(|| rest.strip_prefix(") "))?;
    Some((&line[..digits + 1], after))
}

/// Splits one block's text into spans.
pub fn spans(text: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut plain = String::new();
    let mut rest = text;
    while let Some(next) = rest.chars().next() {
        let marked = match next {
            '`' => code_span(rest).map(|(inner, after)| (Span::Code(inner.to_owned()), after)),
            '*' if rest.starts_with("**") => {
                enclosed(rest, "**").map(|(inner, after)| (Span::Bold(inner.to_owned()), after))
            }
            '_' if rest.starts_with("__") => {
                enclosed(rest, "__").map(|(inner, after)| (Span::Bold(inner.to_owned()), after))
            }
            '*' => {
                enclosed(rest, "*").map(|(inner, after)| (Span::Italic(inner.to_owned()), after))
            }
            _ => None,
        };
        match marked {
            Some((span, after)) => {
                if !plain.is_empty() {
                    spans.push(Span::Plain(std::mem::take(&mut plain)));
                }
                spans.push(span);
                rest = after;
            }
            None => {
                // A run of backticks that opens nothing stays as written.
                let skip = match next {
                    '`' => backticks(rest),
                    _ => next.len_utf8(),
                };
                plain.push_str(&rest[..skip]);
                rest = &rest[skip..];
            }
        }
    }
    if !plain.is_empty() {
        spans.push(Span::Plain(plain));
    }
    spans
}

/// When `text` starts with inline code: a run of backticks, some text and
/// a run of as many backticks on the same line, such as `` `x` `` or
/// ```` ```x``` ````. Returns that text and what follows.
fn code_span(text: &str) -> Option<(&str, &str)> {
    let ticks = backticks(text);
    let body = &text[ticks..];
    let mut from = 0;
    while let Some(found) = body[from..].find('`') {
        let start = from + found;
        let run = backticks(&body[start..]);
        if run == ticks {
            let inner = &body[..start];
            return (!inner.is_empty() && !inner.contains('\n'))
                .then(|| (inner, &body[start + run..]));
        }
        from = start + run;
    }
    None
}

/// How many bytes of backticks `text` starts with.
fn backticks(text: &str) -> usize {
    text.len() - text.trim_start_matches('`').len()
}

/// When `text` starts with `marker`, some text, and `marker` again on the
/// same line: that text and what follows. Emphasis must hug its text, so
/// `2 * 3 * 4` stays plain.
fn enclosed<'a>(text: &'a str, marker: &str) -> Option<(&'a str, &'a str)> {
    let body = text.strip_prefix(marker)?;
    let end = body.find(marker)?;
    let inner = &body[..end];
    if inner.is_empty() || inner.contains('\n') {
        return None;
    }
    let hugs = !inner.starts_with(char::is_whitespace) && !inner.ends_with(char::is_whitespace);
    if !hugs {
        return None;
    }
    Some((inner, &body[end + marker.len()..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(text: &str) -> Span {
        Span::Plain(text.to_owned())
    }

    #[test]
    fn paragraphs_split_on_blank_lines_and_keep_line_breaks() {
        assert_eq!(
            parse("One\ntwo\n\nThree"),
            vec![
                Block::Paragraph(vec![plain("One\ntwo")]),
                Block::Paragraph(vec![plain("Three")]),
            ]
        );
        assert_eq!(parse(""), vec![]);
        assert_eq!(parse("\n\n"), vec![]);
    }

    #[test]
    fn inline_code_bold_and_italic() {
        assert_eq!(
            spans("Set `opacity` to **0** and *then* fade"),
            vec![
                plain("Set "),
                Span::Code("opacity".into()),
                plain(" to "),
                Span::Bold("0".into()),
                plain(" and "),
                Span::Italic("then".into()),
                plain(" fade"),
            ]
        );
        assert_eq!(spans("__strong__"), vec![Span::Bold("strong".into())]);
    }

    #[test]
    fn unclosed_or_loose_markers_stay_plain() {
        assert_eq!(spans("a `b"), vec![plain("a `b")]);
        assert_eq!(spans("2 * 3 * 4"), vec![plain("2 * 3 * 4")]);
        assert_eq!(spans("**half"), vec![plain("**half")]);
        assert_eq!(spans("snake_case_name"), vec![plain("snake_case_name")]);
        assert_eq!(spans("` a `"), vec![Span::Code(" a ".into())]);
    }

    #[test]
    fn headings_and_lists() {
        assert_eq!(
            parse("## Plan\n- Fade in\n  - the title\n2. Then **move**\n10) last"),
            vec![
                Block::Heading(vec![plain("Plan")]),
                Block::Bullet {
                    depth: 0,
                    spans: vec![plain("Fade in")],
                },
                Block::Bullet {
                    depth: 1,
                    spans: vec![plain("the title")],
                },
                Block::Numbered {
                    marker: "2.".into(),
                    spans: vec![plain("Then "), Span::Bold("move".into())],
                },
                Block::Numbered {
                    marker: "10)".into(),
                    spans: vec![plain("last")],
                },
            ]
        );
        assert_eq!(
            parse("#hashtag"),
            vec![Block::Paragraph(vec![plain("#hashtag")])]
        );
        assert_eq!(
            parse("2024 was"),
            vec![Block::Paragraph(vec![plain("2024 was")])]
        );
    }

    #[test]
    fn code_blocks_are_kept_as_written_even_unclosed() {
        assert_eq!(
            parse("Here:\n```json\n{ \"x\": 1 }\n  **not bold**\n```\nDone."),
            vec![
                Block::Paragraph(vec![plain("Here:")]),
                Block::Code("{ \"x\": 1 }\n  **not bold**".into()),
                Block::Paragraph(vec![plain("Done.")]),
            ]
        );
        assert_eq!(
            parse("```\nstill streaming"),
            vec![Block::Code("still streaming".into())]
        );
    }

    #[test]
    fn a_line_that_starts_with_inline_code_is_not_a_code_block() {
        assert_eq!(
            parse("```opacity: 0``` fades it out.\nThen move it.\n\nAll done."),
            vec![
                Block::Paragraph(vec![
                    Span::Code("opacity: 0".into()),
                    plain(" fades it out.\nThen move it."),
                ]),
                Block::Paragraph(vec![plain("All done.")]),
            ]
        );
        assert_eq!(
            parse("```bash npm i```"),
            vec![Block::Paragraph(vec![Span::Code("bash npm i".into())])]
        );
    }

    #[test]
    fn inline_code_closes_with_as_many_backticks_as_it_opened() {
        assert_eq!(
            spans("``a`b`` and `c`"),
            vec![
                Span::Code("a`b".into()),
                plain(" and "),
                Span::Code("c".into()),
            ]
        );
        assert_eq!(spans("``half` open"), vec![plain("``half` open")]);
    }
}
