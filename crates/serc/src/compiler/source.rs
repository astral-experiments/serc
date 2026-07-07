// Compiler submodules intentionally share the parent-owned state and lowering vocabulary.
#![allow(clippy::wildcard_imports)]

use super::*;

impl Compiler {
    pub(super) fn line_number(&self, offset: u32) -> u32 {
        self.source_line(self.source_offset(offset)).number
    }

    pub(super) fn source_location(&self, range: ruff_text_size::TextRange) -> SourceLocation {
        let (line, column) = self.source_position(u32::from(range.start()));
        let (end_line, end_column) = self.source_position(u32::from(range.end()));
        SourceLocation::new(line, end_line, column, end_column)
    }

    pub(super) fn source_location_including_trailing_semicolon(
        &self,
        range: ruff_text_size::TextRange,
    ) -> SourceLocation {
        let (line, column) = self.source_position(u32::from(range.start()));
        let (end_line, end_column) = self.definition_end_position(range);
        SourceLocation::new(line, end_line, column, end_column)
    }

    pub(super) fn definition_location(
        &self,
        range: ruff_text_size::TextRange,
        name_range: ruff_text_size::TextRange,
        keyword: TokenKind,
        is_async: bool,
    ) -> SourceLocation {
        let tokens = self.context.tokens.in_range(ruff_text_size::TextRange::new(
            range.start(),
            name_range.start(),
        ));
        let start = tokens
            .iter()
            .enumerate()
            .rfind(|(_, token)| token.kind() == keyword)
            .map_or(range.start(), |(keyword_index, keyword)| {
                if is_async {
                    tokens[..keyword_index]
                        .iter()
                        .rfind(|token| token.kind() == TokenKind::Async)
                        .map_or(keyword.start(), ruff_text_size::Ranged::start)
                } else {
                    keyword.start()
                }
            });
        let (line, column) = self.source_position(u32::from(start));
        let (end_line, end_column) = self.definition_end_position(range);
        SourceLocation::new(line, end_line, column, end_column)
    }

    fn definition_end_position(&self, range: ruff_text_size::TextRange) -> (i32, i32) {
        let end = self
            .context
            .tokens
            .after(range.end())
            .first()
            .filter(|token| token.kind() == TokenKind::Semi)
            .map_or(range.end(), ruff_text_size::Ranged::end);
        self.source_position(u32::from(end))
    }

    pub(super) fn source_position(&self, offset: u32) -> (i32, i32) {
        let offset = self.source_offset(offset);
        let line = self.source_line(offset);
        (
            i32::try_from(line.number).unwrap_or(i32::MAX),
            i32::try_from(u32::from(offset - line.start)).unwrap_or(i32::MAX),
        )
    }

    fn source_line(&self, offset: TextSize) -> CachedSourceLine {
        if let Some(line) = self.cached_source_line.get()
            && line.contains(offset)
        {
            return line;
        }

        let line = self.context.line_index.line_index(offset);
        let line_index = line.to_zero_indexed();
        let line_starts = self.context.line_index.line_starts();
        let start = line_starts[line_index];
        let end = line_starts
            .get(line_index + 1)
            .copied()
            .unwrap_or_else(|| self.source_offset(u32::MAX));
        let cached = CachedSourceLine {
            number: u32::try_from(line.get()).unwrap_or(u32::MAX),
            start,
            end,
            is_last: line_index + 1 == line_starts.len(),
        };
        self.cached_source_line.set(Some(cached));
        cached
    }

    fn source_offset(&self, offset: u32) -> TextSize {
        TextSize::new(offset.min(u32::try_from(self.context.source.len()).unwrap_or(u32::MAX)))
    }

    fn source_text(&self, range: ruff_text_size::TextRange) -> &str {
        let start = usize::from(range.start());
        let end = usize::from(range.end());
        &self.context.source[start..end]
    }

    pub(super) fn string_literal_constant(
        &self,
        string: &ruff_python_ast::ExprStringLiteral,
    ) -> Constant {
        let mut bytes = Vec::new();
        let mut has_surrogates = false;
        for part in &string.value {
            if let Some(part_bytes) =
                explicit_surrogate_string(part.as_str(), self.source_text(part.range))
            {
                bytes.extend(part_bytes);
                has_surrogates = true;
            } else {
                bytes.extend_from_slice(part.as_str().as_bytes());
            }
        }
        if has_surrogates {
            Constant::SurrogateString(bytes)
        } else {
            Constant::String(string.value.to_str().to_string())
        }
    }

    pub(super) fn generator_expression_range(
        &self,
        generator: &ruff_python_ast::ExprGenerator,
    ) -> ruff_text_size::TextRange {
        if generator.parenthesized {
            return generator.range;
        }

        ruff_python_ast::token::parentheses_iterator(generator.into(), None, &self.context.tokens)
            .next()
            .unwrap_or(generator.range)
    }

    fn interpolated_string_content_range(
        &self,
        range: ruff_text_size::TextRange,
    ) -> ruff_text_size::TextRange {
        let start = usize::from(range.start());
        let end = usize::from(range.end());
        let text = &self.context.source.as_bytes()[start..end];
        let Some(quote_offset) = text.iter().position(|byte| matches!(byte, b'\'' | b'"')) else {
            return range;
        };
        let quote = text[quote_offset];
        let quote_len = if text.get(quote_offset..quote_offset + 3) == Some(&[quote; 3]) {
            3
        } else {
            1
        };
        let content_start = start + quote_offset + quote_len;
        let has_closing_quote = text.len() >= quote_len
            && text[text.len() - quote_len..]
                .iter()
                .all(|byte| *byte == quote);
        let content_end = if has_closing_quote {
            end - quote_len
        } else {
            end
        };
        ruff_text_size::TextRange::new(
            ruff_text_size::TextSize::new(u32::try_from(content_start).unwrap_or(u32::MAX)),
            ruff_text_size::TextSize::new(u32::try_from(content_end).unwrap_or(u32::MAX)),
        )
    }

    pub(super) fn fstring_result_range(
        &self,
        fstring: &ruff_python_ast::ExprFString,
    ) -> ruff_text_size::TextRange {
        let mut component_count = 0;
        let mut pending_literal = false;
        let mut only_interpolation = None;
        for part in &fstring.value {
            match part {
                FStringPart::Literal(literal) => {
                    pending_literal |= !literal.value.is_empty();
                }
                FStringPart::FString(fstring) => {
                    for element in &fstring.elements {
                        match element {
                            InterpolatedStringElement::Literal(literal) => {
                                pending_literal |= !literal.value.is_empty();
                            }
                            InterpolatedStringElement::Interpolation(interpolation) => {
                                if interpolation.debug_text.is_some() {
                                    pending_literal = true;
                                }
                                if std::mem::take(&mut pending_literal) {
                                    component_count += 1;
                                }
                                component_count += 1;
                                only_interpolation = Some(interpolation.range);
                            }
                        }
                    }
                }
            }
        }
        if pending_literal {
            component_count += 1;
            only_interpolation = None;
        }
        if component_count == 1 {
            only_interpolation.unwrap_or_else(|| {
                fstring_literal_span(fstring)
                    .unwrap_or_else(|| self.interpolated_string_content_range(fstring.range))
            })
        } else {
            fstring.range
        }
    }

    pub(super) fn attribute_opcode_range(
        &self,
        attribute: &ruff_python_ast::ExprAttribute,
    ) -> ruff_text_size::TextRange {
        let location = self.source_location(attribute.range);
        if location.line == location.end_line {
            attribute.range
        } else {
            attribute.attr.range()
        }
    }

    pub(super) fn call_opcode_range(
        &self,
        call: &ruff_python_ast::ExprCall,
    ) -> ruff_text_size::TextRange {
        if let Some(attribute) = self.direct_method_attribute(call) {
            let attribute_location = self.source_location(attribute.range);
            if attribute_location.line != attribute_location.end_line {
                return ruff_text_size::TextRange::new(
                    attribute.attr.range().start(),
                    call.range.end(),
                );
            }
        }
        call.range
    }

    pub(super) fn direct_method_attribute<'a>(
        &self,
        call: &'a ruff_python_ast::ExprCall,
    ) -> Option<&'a ruff_python_ast::ExprAttribute> {
        if call.arguments.args.iter().any(Expr::is_starred_expr)
            || call
                .arguments
                .keywords
                .iter()
                .any(|keyword| keyword.arg.is_none())
            || call.arguments.keywords.len() > 15
            || call.arguments.args.len() + call.arguments.keywords.len() >= 30
        {
            return None;
        }
        let Expr::Attribute(attribute) = call.func.as_ref() else {
            return None;
        };
        (!self.imported_module_attribute(attribute)).then_some(attribute)
    }
}

pub(super) fn debug_text_range(
    interpolation: &ruff_python_ast::InterpolatedElement,
) -> ruff_text_size::TextRange {
    let debug = interpolation
        .debug_text
        .as_ref()
        .expect("debug text range requires debug text");
    let start = u32::from(interpolation.range.start()).saturating_add(1);
    let length = debug.leading().len() + debug.expression().len() + debug.trailing().len();
    let end = start.saturating_add(u32::try_from(length).unwrap_or(u32::MAX));
    ruff_text_size::TextRange::new(
        ruff_text_size::TextSize::new(start),
        ruff_text_size::TextSize::new(end),
    )
}

fn fstring_literal_span(
    fstring: &ruff_python_ast::ExprFString,
) -> Option<ruff_text_size::TextRange> {
    let mut first = None;
    let mut last = None;
    let mut include = |range: ruff_text_size::TextRange| {
        first.get_or_insert(range.start());
        last = Some(range.end());
    };
    for part in &fstring.value {
        match part {
            FStringPart::Literal(literal) => include(literal.range),
            FStringPart::FString(fstring) => {
                for element in &fstring.elements {
                    match element {
                        InterpolatedStringElement::Literal(literal) => include(literal.range),
                        InterpolatedStringElement::Interpolation(_) => return None,
                    }
                }
            }
        }
    }
    Some(ruff_text_size::TextRange::new(first?, last?))
}

/// Removes Python comments while preserving the whitespace and newlines around
/// them, as CPython does for the source text stored by a t-string interpolation.
pub(super) fn strip_expression_comments(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    let mut quote = None;
    let mut triple_quoted = false;

    while index < bytes.len() {
        if let Some(delimiter) = quote {
            if bytes[index] == b'\\' {
                output.push(bytes[index]);
                index += 1;
                if index < bytes.len() {
                    output.push(bytes[index]);
                    index += 1;
                }
                continue;
            }
            if bytes[index] == delimiter {
                if triple_quoted {
                    if bytes.get(index..index + 3) == Some(&[delimiter; 3]) {
                        output.extend_from_slice(&bytes[index..index + 3]);
                        index += 3;
                        quote = None;
                        triple_quoted = false;
                        continue;
                    }
                } else {
                    output.push(bytes[index]);
                    index += 1;
                    quote = None;
                    continue;
                }
            }
            output.push(bytes[index]);
            index += 1;
            continue;
        }

        match bytes[index] {
            b'#' => {
                while index < bytes.len() && !matches!(bytes[index], b'\n' | b'\r') {
                    index += 1;
                }
            }
            delimiter @ (b'\'' | b'"') => {
                triple_quoted = bytes.get(index..index + 3) == Some(&[delimiter; 3]);
                let length = if triple_quoted { 3 } else { 1 };
                output.extend_from_slice(&bytes[index..index + length]);
                index += length;
                quote = Some(delimiter);
            }
            byte => {
                output.push(byte);
                index += 1;
            }
        }
    }

    String::from_utf8(output).expect("removing ASCII comments preserves UTF-8")
}
pub(super) fn implicit_return_range(
    compiler: &Compiler,
    statement: &Stmt,
) -> ruff_text_size::TextRange {
    match statement {
        Stmt::Assign(assignment) => assignment
            .targets
            .last()
            .map_or(statement.range(), |target| {
                last_store_target_range(compiler, target)
            }),
        Stmt::Expr(expression) => match expression.value.as_ref() {
            Expr::FString(fstring) => compiler.fstring_result_range(fstring),
            expression => expression.range(),
        },
        Stmt::AnnAssign(assignment) => {
            let Expr::Subscript(subscript) = assignment.target.as_ref() else {
                return statement.range();
            };
            last_discarded_annotation_slice_range(&subscript.slice)
        }
        Stmt::While(statement) => statement.test.range(),
        Stmt::For(statement) => statement.iter.range(),
        _ => statement.range(),
    }
}

fn last_store_target_range(compiler: &Compiler, expression: &Expr) -> ruff_text_size::TextRange {
    match expression {
        Expr::List(list) => list.elts.last().map_or(expression.range(), |target| {
            last_store_target_range(compiler, target)
        }),
        Expr::Tuple(tuple) => tuple.elts.last().map_or(expression.range(), |target| {
            last_store_target_range(compiler, target)
        }),
        Expr::Starred(starred) => last_store_target_range(compiler, &starred.value),
        Expr::Attribute(attribute) => compiler.attribute_opcode_range(attribute),
        _ => expression.range(),
    }
}

pub(super) fn statement_execution_range(statement: &Stmt) -> ruff_text_size::TextRange {
    match statement {
        Stmt::Assign(statement) => first_expression_instruction_range(&statement.value),
        Stmt::AnnAssign(statement) => statement
            .value
            .as_deref()
            .map_or(statement.range, first_expression_instruction_range),
        Stmt::AugAssign(statement) => first_expression_instruction_range(&statement.target),
        Stmt::Expr(statement) => first_expression_instruction_range(&statement.value),
        Stmt::Return(statement) => statement
            .value
            .as_deref()
            .map_or(statement.range, first_expression_instruction_range),
        _ => statement.range(),
    }
}

fn first_expression_instruction_range(expression: &Expr) -> ruff_text_size::TextRange {
    if matches!(
        expression,
        Expr::BinOp(_) | Expr::Name(_) | Expr::UnaryOp(_) | Expr::Tuple(_)
    ) && fold_constant(expression).is_some()
    {
        return expression.range();
    }
    match expression {
        Expr::Attribute(attribute) => first_expression_instruction_range(&attribute.value),
        Expr::BinOp(binary) => first_expression_instruction_range(&binary.left),
        Expr::BoolOp(boolean) => boolean
            .values
            .first()
            .map_or(expression.range(), first_expression_instruction_range),
        Expr::Call(call) => first_expression_instruction_range(&call.func),
        Expr::Compare(compare) => first_expression_instruction_range(&compare.left),
        Expr::If(conditional) => first_expression_instruction_range(&conditional.test),
        Expr::Named(named) => first_expression_instruction_range(&named.value),
        Expr::Subscript(subscript) => first_expression_instruction_range(&subscript.value),
        Expr::UnaryOp(unary) => first_expression_instruction_range(&unary.operand),
        _ => expression.range(),
    }
}

fn last_discarded_annotation_slice_range(expression: &Expr) -> ruff_text_size::TextRange {
    match expression {
        Expr::Tuple(tuple) => tuple
            .elts
            .last()
            .map_or(expression.range(), last_discarded_annotation_slice_range),
        Expr::Slice(slice) => slice
            .step
            .as_deref()
            .or(slice.upper.as_deref())
            .or(slice.lower.as_deref())
            .map_or(expression.range(), last_discarded_annotation_slice_range),
        _ => expression.range(),
    }
}
pub(super) fn unparse_annotation(expression: &Expr) -> String {
    if let Expr::StringLiteral(string) = expression {
        return python_string_repr(string.value.to_str());
    }
    let indentation = Indentation::default();
    let mut annotation = Generator::new(&indentation, LineEnding::Lf)
        .with_mode(CodegenMode::AstUnparse)
        .expr(expression);
    let bytes = annotation.as_bytes();
    let mut stack = Vec::<(u8, usize)>::new();
    let mut parenthesis_pairs = FxHashSet::default();
    let mut bracket_pairs = Vec::new();
    let mut quote = None::<(u8, bool)>;
    let mut index = 0_usize;
    while index < bytes.len() {
        if let Some((delimiter, triple)) = quote {
            if bytes[index] == b'\\' {
                index = (index + 2).min(bytes.len());
                continue;
            }
            if bytes[index] == delimiter
                && (!triple
                    || bytes.get(index + 1) == Some(&delimiter)
                        && bytes.get(index + 2) == Some(&delimiter))
            {
                index += if triple { 3 } else { 1 };
                quote = None;
                continue;
            }
            index += 1;
            continue;
        }
        match bytes[index] {
            delimiter @ (b'\'' | b'"') => {
                let triple = bytes.get(index + 1) == Some(&delimiter)
                    && bytes.get(index + 2) == Some(&delimiter);
                quote = Some((delimiter, triple));
                index += if triple { 3 } else { 1 };
                continue;
            }
            delimiter @ (b'(' | b'[' | b'{') => stack.push((delimiter, index)),
            b')' | b']' | b'}' => {
                let closing = bytes[index];
                let expected = match closing {
                    b')' => b'(',
                    b']' => b'[',
                    _ => b'{',
                };
                if let Some((opening, opening_index)) = stack.pop()
                    && opening == expected
                {
                    if opening == b'(' {
                        parenthesis_pairs.insert((opening_index, index));
                    } else if opening == b'[' {
                        bracket_pairs.push((opening_index, index));
                    }
                }
            }
            _ => {}
        }
        index += 1;
    }
    let mut removals = bracket_pairs
        .into_iter()
        .filter_map(|(opening, closing)| {
            let is_subscript = annotation.as_bytes()[..opening]
                .iter()
                .rev()
                .find(|byte| !byte.is_ascii_whitespace())
                .is_some_and(|byte| {
                    byte.is_ascii_alphanumeric()
                        || matches!(byte, b'_' | b')' | b']' | b'}' | b'\'' | b'"')
                });
            (is_subscript
                && closing.saturating_sub(opening) > 3
                && parenthesis_pairs.contains(&(opening + 1, closing.saturating_sub(1))))
            .then_some([opening + 1, closing - 1])
        })
        .flatten()
        .collect::<Vec<_>>();
    removals.sort_unstable_by(|left, right| right.cmp(left));
    for index in removals {
        annotation.remove(index);
    }
    annotation
}

fn python_string_repr(value: &str) -> String {
    let quote = if value.contains('\'') && !value.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut repr = String::with_capacity(value.len() + 2);
    repr.push(quote);
    for character in value.chars() {
        match character {
            '\\' => repr.push_str("\\\\"),
            '\n' => repr.push_str("\\n"),
            '\r' => repr.push_str("\\r"),
            '\t' => repr.push_str("\\t"),
            '\x08' => repr.push_str("\\b"),
            '\x0c' => repr.push_str("\\f"),
            character if character == quote => {
                repr.push('\\');
                repr.push(character);
            }
            character if character.is_control() => {
                use std::fmt::Write;
                write!(repr, "\\x{:02x}", u32::from(character)).unwrap();
            }
            character => repr.push(character),
        }
    }
    repr.push(quote);
    repr
}
