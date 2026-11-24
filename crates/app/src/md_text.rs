// Adapted from iced's `widget/src/text/rich.rs` (MIT, https://github.com/iced-rs/iced, as pinned
// by libcosmic): rich text, plus a mouse selection held by the app (F3 Markdown).

//! A block of F3 Markdown (a paragraph, a heading, a code line) drawn as rich text that can be
//! selected. The selection lives in `Lister::sel` as (block, char): this widget reports where
//! the button went down and where it is dragged to, and paints the part of it in its own block,
//! so one selection runs across blocks.

use crate::app::Message;
use cosmic::Renderer;
use cosmic::iced::core::text::{Affinity, Paragraph as _, Span};
use cosmic::iced::core::widget::text::{self, Catalog, LineHeight, Shaping, Wrapping};
use cosmic::iced::core::widget::tree::{self, Tree};
use cosmic::iced::core::{
    Clipboard, Color, Element, Layout, Length, Pixels, Point, Rectangle, Shell, Size, Vector,
    Widget, layout, mouse, renderer, renderer::Renderer as _, text::Renderer as _,
};
use cosmic::iced::{Event, Font, alignment};
use std::ops::Range;
use std::sync::Arc;

type Para = <Renderer as cosmic::iced::core::text::Renderer>::Paragraph;
type Spans = Arc<[Span<'static, String, Font>]>;

/// Block `block` of the page; `sel`: its selected chars; `dragging`: the button is down.
pub struct MdText {
    spans: Spans,
    size: Pixels,
    font: Option<Font>,
    block: usize,
    sel: Option<Range<usize>>,
    dragging: bool,
    on_link: fn(String) -> Message,
}

pub fn md_text(
    spans: Spans,
    size: impl Into<Pixels>,
    block: usize,
    sel: Option<Range<usize>>,
    dragging: bool,
    on_link: fn(String) -> Message,
) -> MdText {
    MdText {
        spans,
        size: size.into(),
        font: None,
        block,
        sel,
        dragging,
        on_link,
    }
}

impl MdText {
    pub fn font(mut self, font: Font) -> Self {
        self.font = Some(font);
        self
    }
}

/// The block's text, as selected and copied: its spans in a row.
pub fn plain(spans: &[Span<'_, String, Font>]) -> String {
    spans.iter().map(|s| s.text.as_ref()).collect()
}

#[derive(Default)]
struct State {
    spans: Vec<Span<'static, String, Font>>,
    paragraph: Para,
    hovered_link: Option<usize>,
    /// The link the button went down on, and where: let go there, it opens.
    pressed: Option<(Option<usize>, Point)>,
    /// The last char reported while dragging (each move would be a message).
    reported: Option<usize>,
}

impl Widget<Message, cosmic::Theme, Renderer> for MdText {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Shrink, Length::Shrink)
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let state = tree.state.downcast_mut::<State>();
        let (size, spans) = (self.size, self.spans.as_ref());
        let font = self.font.unwrap_or_else(|| renderer.default_font());
        layout::sized(limits, Length::Shrink, Length::Shrink, |limits| {
            let bounds = limits.max();
            let text = cosmic::iced::core::Text {
                content: spans,
                bounds,
                size,
                line_height: LineHeight::default(),
                font,
                align_x: text::Alignment::Default,
                align_y: alignment::Vertical::Top,
                shaping: Shaping::Advanced,
                wrapping: Wrapping::default(),
                ellipsize: Default::default(),
            };
            if state.spans != spans {
                state.paragraph = Para::with_spans(text);
                state.spans = spans.iter().cloned().map(Span::to_static).collect();
            } else {
                match state.paragraph.compare(text.with_content(())) {
                    cosmic::iced::core::text::Difference::None => {}
                    cosmic::iced::core::text::Difference::Bounds => state.paragraph.resize(bounds),
                    cosmic::iced::core::text::Difference::Shape => {
                        state.paragraph = Para::with_spans(text)
                    }
                }
            }
            state.paragraph.min_bounds()
        })
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &cosmic::Theme,
        defaults: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        if !layout.bounds().intersects(viewport) {
            return;
        }
        let state = tree.state.downcast_ref::<State>();
        let at = layout.position() - Point::ORIGIN;
        let style = theme.style(&<cosmic::Theme as Catalog>::default());

        for (index, span) in self.spans.iter().enumerate() {
            let hovered = Some(index) == state.hovered_link;
            if span.highlight.is_none() && !span.underline && !span.strikethrough && !hovered {
                continue;
            }
            let regions = state.paragraph.span_bounds(index);
            if let Some(highlight) = span.highlight {
                for b in &regions {
                    let pad = Vector::new(span.padding.left, span.padding.top);
                    let quad = renderer::Quad {
                        bounds: Rectangle::new(
                            b.position() - pad,
                            b.size() + Size::new(span.padding.x(), span.padding.y()),
                        ) + at,
                        border: highlight.border,
                        ..Default::default()
                    };
                    renderer.fill_quad(quad, highlight.background);
                }
            }
            let size = span.size.unwrap_or(self.size);
            let line_h = (span.line_height.unwrap_or_default()).to_absolute(size);
            let color = span.color.or(style.color).unwrap_or(defaults.text_color);
            let baseline = at + Vector::new(0.0, size.0 + (line_h.0 - size.0) / 2.0);
            let mut line = |dy: f32| {
                for b in &regions {
                    let quad = renderer::Quad {
                        bounds: Rectangle::new(
                            b.position() + baseline - Vector::new(0.0, dy),
                            Size::new(b.width, 1.0),
                        ),
                        ..Default::default()
                    };
                    renderer.fill_quad(quad, color);
                }
            };
            if span.underline || hovered {
                line(size.0 * 0.08);
            }
            if span.strikethrough {
                line(size.0 / 2.0);
            }
        }

        // Over the spans' own backgrounds (inline code), under the text.
        if let Some(sel) = &self.sel {
            let mut fill: Color = theme.cosmic().accent_color().into();
            fill.a = 0.35;
            let text = plain(&self.spans);
            for (line, lo, hi) in line_ranges(&text, sel.clone()) {
                let rects = (state.paragraph).highlight(
                    line,
                    (lo, Affinity::After),
                    (hi, Affinity::Before),
                );
                for r in rects {
                    let quad = renderer::Quad {
                        bounds: r + at,
                        ..Default::default()
                    };
                    renderer.fill_quad(quad, fill);
                }
            }
        }

        text::draw(
            renderer,
            defaults,
            layout.bounds(),
            &state.paragraph,
            style,
            viewport,
        );
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();
        let bounds = layout.bounds();
        let inside = cursor.position_in(bounds);
        let hovered = inside.and_then(|p| {
            let span = state.paragraph.hit_span(p)?;
            self.spans.get(span)?.link.as_ref().map(|_| span)
        });
        if hovered != state.hovered_link {
            state.hovered_link = hovered;
            shell.request_redraw();
        }
        if !self.dragging {
            state.reported = None;
        }
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if let Some(p) = inside {
                    let at = char_at(&state.paragraph, &self.spans, p);
                    state.pressed = Some((hovered, p));
                    state.reported = Some(at);
                    shell.publish(Message::ListerMdPress(self.block, at));
                    shell.capture_event();
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) if self.dragging => {
                if let Some(p) = inside {
                    let at = char_at(&state.paragraph, &self.spans, p);
                    if state.reported != Some(at) {
                        state.reported = Some(at);
                        shell.publish(Message::ListerMdDrag(self.block, at));
                    }
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                // A click on a link opens it; a drag that started there selects.
                if let (Some((Some(link), from)), Some(p)) = (state.pressed.take(), inside)
                    && hovered == Some(link)
                    && from.distance(p) < 4.0
                    && let Some(url) = self.spans.get(link).and_then(|s| s.link.clone())
                {
                    shell.publish((self.on_link)(url));
                }
            }
            _ => {}
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        _layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        match tree.state.downcast_ref::<State>().hovered_link {
            Some(_) => mouse::Interaction::Pointer,
            None => mouse::Interaction::Text,
        }
    }
}

impl<'a> From<MdText> for Element<'a, Message, cosmic::Theme, Renderer> {
    fn from(t: MdText) -> Self {
        Element::new(t)
    }
}

/// The char of the block's text at `p` (block-relative). A hard break splits the text into
/// lines of their own (`hit_test` counts in a line): the line is the last one starting above.
fn char_at(paragraph: &Para, spans: &[Span<'_, String, Font>], p: Point) -> usize {
    let text = plain(spans);
    let size = paragraph.min_bounds();
    let p = Point::new(
        p.x.clamp(0.0, size.width),
        p.y.clamp(0.0, size.height - 1.0),
    );
    let starts: Vec<usize> = std::iter::once(0)
        .chain(text.match_indices('\n').map(|(i, _)| i + 1))
        .collect();
    let line = (0..starts.len())
        .rev()
        .find(|&l| {
            paragraph
                .grapheme_position(l, 0)
                .is_some_and(|q| q.y <= p.y)
        })
        .unwrap_or(0);
    let byte = paragraph.hit_test(p).map_or(0, |h| h.cursor());
    let end = text[starts[line]..]
        .find('\n')
        .map_or(text.len(), |e| starts[line] + e);
    let byte = (starts[line] + byte).min(end);
    text.get(..byte).map_or(0, |s| s.chars().count())
}

/// The chars `sel` of `text` as (line, start byte, end byte) per line of the text.
fn line_ranges(text: &str, sel: Range<usize>) -> Vec<(usize, usize, usize)> {
    let byte = |c: usize| text.char_indices().nth(c).map_or(text.len(), |(b, _)| b);
    let (lo, hi) = (byte(sel.start), byte(sel.end));
    let mut out = Vec::new();
    let mut start = 0;
    for (line, l) in text.split('\n').enumerate() {
        let end = start + l.len();
        let (a, b) = (lo.max(start), hi.min(end));
        if a < b {
            out.push((line, a - start, b - start));
        }
        start = end + 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::line_ranges;

    #[test]
    fn a_selection_is_cut_at_hard_breaks() {
        assert_eq!(line_ranges("abc", 1..3), vec![(0, 1, 3)]);
        assert_eq!(line_ranges("ab\ncd", 1..4), vec![(0, 1, 2), (1, 0, 1)]);
        assert_eq!(line_ranges("жж\nя", 1..3), vec![(0, 2, 4)]); // bytes, not chars
    }
}
