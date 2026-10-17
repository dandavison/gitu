use crate::config::Config;
use crate::config::SyntaxHighlightConfig;
use crate::style::Style;
use crate::syntax_parser;
use crate::syntax_parser::SyntaxTag;
use itertools::Itertools;
use std::iter;
use std::iter::Peekable;
use std::ops::Range;
use std::path::Path;

/// Construct a newline inclusive iterator over each line in a chunk of text.
pub fn line_range_iterator(content: &str) -> impl Iterator<Item = (Range<usize>, &str)> {
    content
        .split_inclusive('\n')
        .scan(0usize, |prev_line_end, current_line| {
            let line_start = *prev_line_end;

            let actual_line_length = current_line.len();

            let visual_line_length = if current_line.ends_with("\r\n") {
                actual_line_length - 2
            } else {
                actual_line_length - 1
            };

            let actual_line_end = line_start + actual_line_length;

            let visual_line_end = line_start + visual_line_length;

            *prev_line_end = actual_line_end;

            Some((line_start..visual_line_end, current_line))
        })
}

pub(crate) fn iter_syntax_highlights<'a>(
    config: &'a SyntaxHighlightConfig,
    path: &'a str,
    content: String,
) -> Peekable<impl Iterator<Item = (Range<usize>, Style)> + 'a> {
    fill_gaps(
        0..content.len(),
        if config.enabled {
            syntax_parser::parse(Path::new(path), &content)
        } else {
            vec![]
        }
        .into_iter()
        .map(move |(range, tag)| (range, syntax_highlight_tag_style(config, tag))),
        Style::new(),
    )
    .peekable()
}

pub(crate) fn highlight_blame_file(
    config: &Config,
    file_path: &str,
    content: String,
) -> BlameHighlights {
    let mut highlights_iter =
        iter_syntax_highlights(&config.style.syntax_highlight, file_path, content.clone());

    let mut result = BlameHighlights {
        spans: vec![],
        line_index: vec![],
    };

    for (line_range, _) in line_range_iterator(&content) {
        let start = result.spans.len();
        collect_line_highlights(&mut highlights_iter, &line_range, &mut result.spans);
        result.line_index.push(start..result.spans.len());
    }

    result
}

#[derive(Debug, Clone)]
pub struct BlameHighlights {
    spans: Vec<(Range<usize>, Style)>,
    line_index: Vec<Range<usize>>,
}

impl BlameHighlights {
    pub fn get_line_highlights(&self, line: usize) -> &[(Range<usize>, Style)] {
        if line >= self.line_index.len() {
            return &[];
        }
        &self.spans[self.line_index[line].clone()]
    }
}

pub(crate) fn fill_gaps<T: Clone + Default>(
    full_range: Range<usize>,
    ranges: impl Iterator<Item = (Range<usize>, T)>,
    fill: T,
) -> impl Iterator<Item = (Range<usize>, T)> {
    iter::once((full_range.start, None))
        .chain(ranges.flat_map(|(range, item)| vec![(range.start, Some(item)), (range.end, None)]))
        .chain([(full_range.end, None)])
        .tuple_windows()
        .map(move |((start, item_a), (end, _))| (start..end, item_a.unwrap_or(fill.clone())))
        .filter(|(range, _)| !range.is_empty())
        .peekable()
}

pub(crate) fn collect_line_highlights(
    highlights_iter: &mut Peekable<impl Iterator<Item = (Range<usize>, Style)>>,
    line_range: &Range<usize>,
    result: &mut Vec<(Range<usize>, Style)>,
) {
    while let Some((range, style)) = highlights_iter.peek() {
        // if the current range in the iter ends before the line we
        // are interested in highlights for, we advance the iterator
        // and continue the loop
        if range.end <= line_range.start {
            highlights_iter.next();
            continue;
        }

        // clamp the range to within the given line range
        let start = range.start.max(line_range.start);
        let end = range.end.min(line_range.end);

        // the range coordinates have to be localized to the line in question
        // before we report the highlights
        let local_line_range_start = start - line_range.start;
        let local_line_range_end = end - line_range.start;

        result.push((local_line_range_start..local_line_range_end, *style));

        // break loop if we are outside of the line range
        if line_range.end <= range.end {
            break;
        }

        highlights_iter.next();
    }
}

pub(crate) fn syntax_highlight_tag_style(config: &SyntaxHighlightConfig, tag: SyntaxTag) -> Style {
    match tag {
        SyntaxTag::Attribute => &config.attribute,
        SyntaxTag::Comment => &config.comment,
        SyntaxTag::Constant => &config.constant,
        SyntaxTag::ConstantBuiltin => &config.constant_builtin,
        SyntaxTag::Constructor => &config.constructor,
        SyntaxTag::Embedded => &config.embedded,
        SyntaxTag::Function => &config.function,
        SyntaxTag::FunctionBuiltin => &config.function_builtin,
        SyntaxTag::Keyword => &config.keyword,
        SyntaxTag::Module => &config.module,
        SyntaxTag::Number => &config.number,
        SyntaxTag::Operator => &config.operator,
        SyntaxTag::Property => &config.property,
        SyntaxTag::PunctuationBracket => &config.punctuation_bracket,
        SyntaxTag::PunctuationDelimiter => &config.punctuation_delimiter,
        SyntaxTag::String => &config.string,
        SyntaxTag::StringSpecial => &config.string_special,
        SyntaxTag::Tag => &config.tag,
        SyntaxTag::TypeBuiltin => &config.type_builtin,
        SyntaxTag::TypeRegular => &config.type_regular,
        SyntaxTag::VariableBuiltin => &config.variable_builtin,
        SyntaxTag::VariableParameter => &config.variable_parameter,
    }
    .into()
}
