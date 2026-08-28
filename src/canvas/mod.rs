pub mod geometry;
pub mod render;
pub mod widget;

pub use geometry::{Bounds, Pos};
use image::Rgba;
pub use widget::WidgetKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolKind {
    Pencil,
    Highlighter,
    Line,
    Arrow,
    Rect,
    Ellipse,
    Pixelate,
    Counter,
    Spotlight,
    Widget(WidgetKind),
}

impl ToolKind {
    pub const ALL: [ToolKind; 15] = [
        ToolKind::Pencil,
        ToolKind::Highlighter,
        ToolKind::Line,
        ToolKind::Arrow,
        ToolKind::Rect,
        ToolKind::Ellipse,
        ToolKind::Pixelate,
        ToolKind::Counter,
        ToolKind::Spotlight,
        ToolKind::Widget(WidgetKind::ALL[0]),
        ToolKind::Widget(WidgetKind::ALL[1]),
        ToolKind::Widget(WidgetKind::ALL[2]),
        ToolKind::Widget(WidgetKind::ALL[3]),
        ToolKind::Widget(WidgetKind::ALL[4]),
        ToolKind::Widget(WidgetKind::ALL[5]),
    ];

    /// Drag-to-size widget stamp, if any.
    pub fn widget_kind(self) -> Option<WidgetKind> {
        match self {
            ToolKind::Widget(k) => Some(k),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Style {
    pub color: Rgba<u8>,
    pub width: f32,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            color: Rgba([255, 50, 50, 255]),
            width: 4.0,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Annotation {
    Pencil {
        points: Vec<Pos>,
        color: Rgba<u8>,
        width: f32,
    },
    Line {
        start: Pos,
        end: Pos,
        color: Rgba<u8>,
        width: f32,
    },
    Arrow {
        start: Pos,
        end: Pos,
        color: Rgba<u8>,
        width: f32,
    },
    Rect {
        rect: Bounds,
        color: Rgba<u8>,
        width: f32,
    },
    Ellipse {
        rect: Bounds,
        color: Rgba<u8>,
        width: f32,
    },
    Pixelate {
        rect: Bounds,
        block: u32,
    },
    /// Axis-aligned rect that stays at full brightness; the rest of the crop
    /// is dimmed. No stroke.
    Spotlight {
        rect: Bounds,
    },
    Counter {
        center: Pos,
        number: u32,
        color: Rgba<u8>,
        radius: f32,
    },
    /// Drag-to-size UI mock (button / input / image-X / checkbox / toggle / measure).
    Widget {
        kind: WidgetKind,
        rect: Bounds,
        color: Rgba<u8>,
        width: f32,
    },
}

#[derive(Debug)]
pub struct Canvas {
    pub annotations: Vec<Annotation>,
    pub redo: Vec<Annotation>,
    pub style: Style,
    /// `None` means no drawing tool is armed — inside-drag of the selection
    /// rectangle moves it instead of starting an annotation.
    pub tool: Option<ToolKind>,
    counter: u32,
}

impl Default for Canvas {
    fn default() -> Self {
        Self {
            annotations: Vec::new(),
            redo: Vec::new(),
            style: Style::default(),
            tool: None,
            counter: 0,
        }
    }
}

impl Canvas {
    pub fn push(&mut self, a: Annotation) {
        self.annotations.push(a);
        self.redo.clear();
    }

    pub fn undo(&mut self) {
        if let Some(a) = self.annotations.pop() {
            if matches!(a, Annotation::Counter { .. }) {
                self.counter = self.counter.saturating_sub(1);
            }
            self.redo.push(a);
        }
    }

    pub fn redo(&mut self) {
        if let Some(a) = self.redo.pop() {
            if matches!(a, Annotation::Counter { .. }) {
                self.counter += 1;
            }
            self.annotations.push(a);
        }
    }

    pub fn next_counter(&mut self) -> u32 {
        self.counter += 1;
        self.counter
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn red_style() -> Style {
        Style {
            color: Rgba([255, 0, 0, 255]),
            width: 3.0,
        }
    }

    fn sample_counter(center: Pos, n: u32) -> Annotation {
        Annotation::Counter {
            center,
            number: n,
            color: Rgba([255, 0, 0, 255]),
            radius: 16.0,
        }
    }

    #[test]
    fn toolkind_all_has_15_items() {
        assert_eq!(ToolKind::ALL.len(), 15);
        assert_eq!(ToolKind::ALL[8], ToolKind::Spotlight);
        let widgets: Vec<_> = ToolKind::ALL
            .iter()
            .filter_map(|t| t.widget_kind())
            .collect();
        assert_eq!(widgets.as_slice(), &WidgetKind::ALL);
    }

    #[test]
    fn widget_tools_map_to_kinds() {
        for &kind in &WidgetKind::ALL {
            let tool = ToolKind::Widget(kind);
            assert_eq!(tool.widget_kind(), Some(kind));
        }
        assert_eq!(ToolKind::Rect.widget_kind(), None);
        assert_eq!(ToolKind::Spotlight.widget_kind(), None);
    }

    #[test]
    fn canvas_default_empty() {
        let c = Canvas::default();
        assert!(c.annotations.is_empty());
        assert!(c.redo.is_empty());
        assert!(c.tool.is_none());
        assert_eq!(c.counter, 0);
    }

    #[test]
    fn push_clears_redo_stack() {
        let mut c = Canvas::default();
        let a1 = Annotation::Line {
            start: Pos { x: 0.0, y: 0.0 },
            end: Pos { x: 10.0, y: 10.0 },
            color: red_style().color,
            width: 2.0,
        };
        let a2 = Annotation::Rect {
            rect: Bounds {
                x: 5.0,
                y: 5.0,
                w: 20.0,
                h: 20.0,
            },
            color: red_style().color,
            width: 2.0,
        };

        c.push(a1.clone());
        c.undo();
        assert_eq!(c.redo.len(), 1);

        c.push(a2.clone());
        assert!(c.redo.is_empty(), "redo must be cleared on new push");
        assert_eq!(c.annotations.len(), 1);
    }

    #[test]
    fn undo_redo_basic() {
        let mut c = Canvas::default();
        let a = Annotation::Ellipse {
            rect: Bounds {
                x: 0.0,
                y: 0.0,
                w: 100.0,
                h: 50.0,
            },
            color: red_style().color,
            width: 4.0,
        };

        c.push(a.clone());
        assert_eq!(c.annotations.len(), 1);
        assert_eq!(c.redo.len(), 0);

        c.undo();
        assert!(c.annotations.is_empty());
        assert_eq!(c.redo.len(), 1);

        c.redo();
        assert_eq!(c.annotations.len(), 1);
        assert!(c.redo.is_empty());
    }

    #[test]
    fn undo_redo_multiple() {
        let mut c = Canvas::default();
        for i in 0..3 {
            c.push(sample_counter(
                Pos {
                    x: i as f32,
                    y: 0.0,
                },
                i,
            ));
        }
        assert_eq!(c.annotations.len(), 3);

        c.undo();
        c.undo();
        assert_eq!(c.annotations.len(), 1);
        assert_eq!(c.redo.len(), 2);

        c.redo();
        assert_eq!(c.annotations.len(), 2);
        assert_eq!(c.redo.len(), 1);
    }

    #[test]
    fn counter_increment_and_undo_adjusts_counter() {
        let mut c = Canvas::default();
        assert_eq!(c.next_counter(), 1);
        assert_eq!(c.next_counter(), 2);

        c.push(sample_counter(Pos { x: 0.0, y: 0.0 }, 2));
        assert_eq!(c.counter, 2);

        c.undo();
        assert_eq!(c.counter, 1, "undoing a counter must decrement the counter");

        // Redo should restore the number
        c.redo();
        assert_eq!(c.counter, 2);
    }

    #[test]
    fn undo_non_counter_does_not_touch_counter() {
        let mut c = Canvas::default();
        c.next_counter(); // -> 1

        let line = Annotation::Line {
            start: Pos { x: 0.0, y: 0.0 },
            end: Pos { x: 1.0, y: 1.0 },
            color: red_style().color,
            width: 1.0,
        };
        c.push(line);
        c.undo();
        assert_eq!(c.counter, 1, "non-counter undo must not affect counter");
    }

    #[test]
    fn redo_counter_increments_counter() {
        let mut c = Canvas::default();
        // Push a counter annotation directly (simulating what next_counter + push does)
        c.push(sample_counter(Pos { x: 10.0, y: 10.0 }, 1));
        c.undo();
        assert_eq!(c.counter, 0);

        c.redo();
        assert_eq!(
            c.counter, 1,
            "redo of a counter annotation should increment the live counter"
        );
    }

    #[test]
    fn style_default_is_red_4px() {
        let s = Style::default();
        assert_eq!(s.color.0, [255, 50, 50, 255]);
        assert_eq!(s.width, 4.0);
    }
}
