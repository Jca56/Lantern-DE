/// Rich text formatting attributes for a span of text.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextAttrs {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    /// Override font size in logical pixels. `None` = use editor default.
    pub font_size: Option<f32>,
    /// Override text color packed as 0xRRGGBB. `None` = use the theme's
    /// default text color. Used by the markdown preview to color inline /
    /// fenced code without going through the syntax highlighter.
    pub color: Option<u32>,
    /// Background fill color packed as 0xRRGGBB. `None` = transparent. The
    /// renderer paints a rounded rect behind the span before drawing text.
    pub bg_color: Option<u32>,
}

impl Default for TextAttrs {
    fn default() -> Self {
        Self {
            bold: false,
            italic: false,
            underline: false,
            strikethrough: false,
            font_size: None,
            color: None,
            bg_color: None,
        }
    }
}

impl TextAttrs {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// A contiguous byte range within a single line that shares formatting.
#[derive(Clone, Debug, PartialEq)]
pub struct FormatSpan {
    pub start: usize,
    pub end: usize,
    pub attrs: TextAttrs,
}

/// Format spans for a single line of text. Spans are sorted, non-overlapping,
/// and cover subsets of `[0, line_len)`. Gaps are implicitly default-formatted.
#[derive(Clone, Debug, Default)]
pub struct LineFormats {
    spans: Vec<FormatSpan>,
}

impl LineFormats {
    pub fn new() -> Self {
        Self { spans: Vec::new() }
    }

    /// Get the formatting at a specific byte offset.
    pub fn attrs_at(&self, offset: usize) -> TextAttrs {
        for span in &self.spans {
            if offset < span.start {
                break;
            }
            if offset >= span.start && offset < span.end {
                return span.attrs;
            }
        }
        TextAttrs::default()
    }

    /// Apply a formatting toggle to the byte range `[start, end)`.
    /// `toggle_fn` mutates the attrs (e.g. `|a| a.bold = !a.bold`).
    pub fn apply_format(&mut self, start: usize, end: usize, toggle_fn: impl Fn(&mut TextAttrs)) {
        if start >= end {
            return;
        }

        // Collect the existing attrs across this range so we can apply the toggle
        let covered = self.collect_attrs_in_range(start, end);
        let mut toggled: Vec<(usize, usize, TextAttrs)> = covered
            .into_iter()
            .map(|(s, e, mut a)| {
                toggle_fn(&mut a);
                (s, e, a)
            })
            .collect();

        // Remove all spans that overlap [start, end), keeping parts outside the range
        let mut kept = Vec::new();
        for span in self.spans.drain(..) {
            if span.end <= start || span.start >= end {
                // Entirely outside — keep as is
                kept.push(span);
            } else {
                // Partially or fully inside — trim the outside parts
                if span.start < start {
                    kept.push(FormatSpan {
                        start: span.start,
                        end: start,
                        attrs: span.attrs,
                    });
                }
                if span.end > end {
                    kept.push(FormatSpan {
                        start: end,
                        end: span.end,
                        attrs: span.attrs,
                    });
                }
            }
        }

        // Add the toggled spans
        for (s, e, attrs) in toggled.drain(..) {
            if !attrs.is_default() {
                kept.push(FormatSpan { start: s, end: e, attrs });
            }
        }

        kept.sort_by_key(|s| s.start);
        self.spans = kept;
        self.merge_adjacent();
    }

    /// Collect the effective attrs for every sub-range within `[start, end)`,
    /// splitting at existing span boundaries.
    fn collect_attrs_in_range(&self, start: usize, end: usize) -> Vec<(usize, usize, TextAttrs)> {
        // Gather all boundary points within [start, end)
        let mut boundaries = vec![start, end];
        for span in &self.spans {
            if span.start > start && span.start < end {
                boundaries.push(span.start);
            }
            if span.end > start && span.end < end {
                boundaries.push(span.end);
            }
        }
        boundaries.sort();
        boundaries.dedup();

        let mut result = Vec::new();
        for pair in boundaries.windows(2) {
            let (s, e) = (pair[0], pair[1]);
            let attrs = self.attrs_at(s);
            result.push((s, e, attrs));
        }
        result
    }

    /// Merge adjacent spans with identical attrs.
    fn merge_adjacent(&mut self) {
        let mut i = 0;
        while i + 1 < self.spans.len() {
            if self.spans[i].end == self.spans[i + 1].start
                && self.spans[i].attrs == self.spans[i + 1].attrs
            {
                self.spans[i].end = self.spans[i + 1].end;
                self.spans.remove(i + 1);
            } else {
                i += 1;
            }
        }
    }

    /// Check if the entire range `[start, end)` uniformly has an attribute.
    /// Returns the "intersection" of all attrs in the range — an attr is true
    /// only if it's true everywhere in the range.
    pub fn query_uniform(&self, start: usize, end: usize) -> TextAttrs {
        if start >= end {
            return TextAttrs::default();
        }
        let segments = self.collect_attrs_in_range(start, end);
        if segments.is_empty() {
            return TextAttrs::default();
        }
        let mut result = segments[0].2;
        for &(_, _, attrs) in &segments[1..] {
            result.bold = result.bold && attrs.bold;
            result.italic = result.italic && attrs.italic;
            result.underline = result.underline && attrs.underline;
            result.strikethrough = result.strikethrough && attrs.strikethrough;
            // Font size: uniform only if all segments agree
            if result.font_size != attrs.font_size {
                result.font_size = None;
            }
        }
        result
    }

    /// Split this line's formats at `byte_offset`. Returns the second half
    /// (offsets shifted to start at 0). Self is mutated to keep only the first half.
    pub fn split_at(&mut self, byte_offset: usize) -> LineFormats {
        let mut left = Vec::new();
        let mut right = Vec::new();

        for span in self.spans.drain(..) {
            if span.end <= byte_offset {
                left.push(span);
            } else if span.start >= byte_offset {
                right.push(FormatSpan {
                    start: span.start - byte_offset,
                    end: span.end - byte_offset,
                    attrs: span.attrs,
                });
            } else {
                // Span crosses the split point
                left.push(FormatSpan {
                    start: span.start,
                    end: byte_offset,
                    attrs: span.attrs,
                });
                right.push(FormatSpan {
                    start: 0,
                    end: span.end - byte_offset,
                    attrs: span.attrs,
                });
            }
        }

        self.spans = left;
        LineFormats { spans: right }
    }

    /// Append another line's formats onto the end of this line.
    /// `self_line_len` is the byte length of this line's text (before the join).
    pub fn append(&mut self, other: LineFormats, self_line_len: usize) {
        for span in other.spans {
            self.spans.push(FormatSpan {
                start: span.start + self_line_len,
                end: span.end + self_line_len,
                attrs: span.attrs,
            });
        }
        self.merge_adjacent();
    }

    /// Shift spans to accommodate an insertion of `len` bytes at `byte_offset`.
    pub fn insert_at(&mut self, byte_offset: usize, len: usize) {
        for span in &mut self.spans {
            if span.start >= byte_offset {
                span.start += len;
                span.end += len;
            } else if span.end > byte_offset {
                // Insertion inside this span — expand it
                span.end += len;
            }
        }
    }

    /// Insert a formatted span for newly typed text at `byte_offset` with length `len`.
    pub fn insert_formatted(&mut self, byte_offset: usize, len: usize, attrs: TextAttrs) {
        // First shift existing spans
        self.insert_at(byte_offset, len);

        // If attrs are non-default, add a span for the new text
        if !attrs.is_default() {
            self.spans.push(FormatSpan {
                start: byte_offset,
                end: byte_offset + len,
                attrs,
            });
            self.spans.sort_by_key(|s| s.start);
            self.merge_adjacent();
        }
    }

    /// Delete the byte range `[start, end)` and shift remaining spans left.
    pub fn delete_range(&mut self, start: usize, end: usize) {
        let len = end - start;
        let mut new_spans = Vec::new();

        for span in &self.spans {
            if span.end <= start {
                // Before the deletion — keep as is
                new_spans.push(span.clone());
            } else if span.start >= end {
                // After the deletion — shift left
                new_spans.push(FormatSpan {
                    start: span.start - len,
                    end: span.end - len,
                    attrs: span.attrs,
                });
            } else {
                // Overlaps the deletion — trim
                let new_start = span.start.min(start);
                let new_end = if span.end <= end {
                    start // span ends within deletion
                } else {
                    span.end - len // span extends past deletion
                };
                if new_start < new_end {
                    new_spans.push(FormatSpan {
                        start: new_start,
                        end: new_end,
                        attrs: span.attrs,
                    });
                }
            }
        }

        self.spans = new_spans;
        self.merge_adjacent();
    }

    /// Iterate spans covering the entire line `[0, line_len)`,
    /// filling gaps with default-formatted spans.
    pub fn iter_spans(&self, line_len: usize) -> Vec<FormatSpan> {
        if line_len == 0 {
            return vec![];
        }

        let mut result = Vec::new();
        let mut pos = 0;

        for span in &self.spans {
            if span.start >= line_len {
                break;
            }
            let span_end = span.end.min(line_len);

            if span.start > pos {
                // Gap before this span — fill with default
                result.push(FormatSpan {
                    start: pos,
                    end: span.start,
                    attrs: TextAttrs::default(),
                });
            }
            result.push(FormatSpan {
                start: span.start,
                end: span_end,
                attrs: span.attrs,
            });
            pos = span_end;
        }

        // Trailing gap
        if pos < line_len {
            result.push(FormatSpan {
                start: pos,
                end: line_len,
                attrs: TextAttrs::default(),
            });
        }

        result
    }
}

/// Per-line format spans for the entire document.
#[derive(Clone, Debug)]
pub struct DocFormats {
    lines: Vec<LineFormats>,
}

impl DocFormats {
    pub fn new(num_lines: usize) -> Self {
        Self {
            lines: (0..num_lines).map(|_| LineFormats::new()).collect(),
        }
    }

    pub fn get(&self, line: usize) -> &LineFormats {
        &self.lines[line]
    }

    pub fn get_mut(&mut self, line: usize) -> &mut LineFormats {
        &mut self.lines[line]
    }

    pub fn insert_line(&mut self, index: usize, formats: LineFormats) {
        self.lines.insert(index, formats);
    }

    pub fn remove_line(&mut self, index: usize) -> LineFormats {
        self.lines.remove(index)
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// Apply a format toggle across a multi-line selection range.
    pub fn apply_format_range(
        &mut self,
        start_line: usize,
        start_col: usize,
        end_line: usize,
        end_col: usize,
        line_lens: &[usize],
        toggle_fn: &dyn Fn(&mut TextAttrs),
    ) {
        if start_line == end_line {
            self.lines[start_line].apply_format(start_col, end_col, toggle_fn);
        } else {
            // First line: from start_col to end of line
            self.lines[start_line].apply_format(start_col, line_lens[start_line], toggle_fn);
            // Middle lines: full lines
            for i in (start_line + 1)..end_line {
                self.lines[i].apply_format(0, line_lens[i], toggle_fn);
            }
            // Last line: from 0 to end_col
            self.lines[end_line].apply_format(0, end_col, toggle_fn);
        }
    }

    /// Query uniform formatting across a multi-line selection range.
    pub fn query_uniform_range(
        &self,
        start_line: usize,
        start_col: usize,
        end_line: usize,
        end_col: usize,
        line_lens: &[usize],
    ) -> TextAttrs {
        if start_line == end_line {
            return self.lines[start_line].query_uniform(start_col, end_col);
        }

        let mut result = self.lines[start_line].query_uniform(start_col, line_lens[start_line]);
        for i in (start_line + 1)..end_line {
            let a = self.lines[i].query_uniform(0, line_lens[i]);
            result.bold = result.bold && a.bold;
            result.italic = result.italic && a.italic;
            result.underline = result.underline && a.underline;
            result.strikethrough = result.strikethrough && a.strikethrough;
            if result.font_size != a.font_size {
                result.font_size = None;
            }
        }
        let a = self.lines[end_line].query_uniform(0, end_col);
        result.bold = result.bold && a.bold;
        result.italic = result.italic && a.italic;
        result.underline = result.underline && a.underline;
        result.strikethrough = result.strikethrough && a.strikethrough;
        if result.font_size != a.font_size {
            result.font_size = None;
        }
        result
    }
}
