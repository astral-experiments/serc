use ruff_python_ast::token::{TokenKind, Tokens};
use ruff_python_ast::visitor::transformer::{Transformer, walk_expr as transform_expr};
use ruff_python_ast::visitor::{Visitor, walk_expr};
use ruff_python_ast::{
    Expr, ExprContext, FStringPart, InterpolatedStringElement, Operator, Stmt, StringLiteralFlags,
    StringLiteralValue, Suite,
};
use ruff_python_codegen::{Generator, Indentation, Mode as CodegenMode};
use ruff_source_file::{LineEnding, LineIndex};
use ruff_text_size::{Ranged, TextLen, TextSize};
use rustc_hash::FxHashSet;

use crate::assembler::SourceLocation;
use crate::code_object::Constant;

use super::constant::{explicit_surrogate_string, fold_constant};
use super::scope::{ModulePlan, ScopeArena};
use super::{CachedSourceLine, Compiler};

#[derive(Debug)]
pub(super) struct CompilationContext<'source> {
    pub(super) filename: String,
    pub(super) source: &'source str,
    pub(super) tokens: &'source Tokens,
    line_index: LineIndex,
    pub(super) scopes: ScopeArena,
    lowering_profile: LoweringProfile,
}

/// Selects which target-specific layout rules may run while lowering a parsed module.
///
/// Ruff accepts some forward syntax that the pinned CPython target rejects. Keeping that case as
/// an explicit profile prevents a source feature from acting as an undocumented global switch
/// throughout the compiler.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LoweringProfile {
    ExactTarget,
    ForwardSyntax,
}

impl LoweringProfile {
    fn for_suite(body: &Suite) -> Self {
        if suite_has_lazy_imports(body) {
            Self::ForwardSyntax
        } else {
            Self::ExactTarget
        }
    }

    const fn target_specific_rules_enabled(self) -> bool {
        matches!(self, Self::ExactTarget)
    }
}

pub(super) struct CallShape<'a> {
    pub(super) positional_count: usize,
    pub(super) keyword_count: usize,
    pub(super) has_starred: bool,
    pub(super) has_keyword_unpack: bool,
    pub(super) direct_method: Option<&'a ruff_python_ast::ExprAttribute>,
}

impl<'source> CompilationContext<'source> {
    pub(super) fn new(
        filename: &str,
        source: &'source str,
        tokens: &'source Tokens,
        body: &Suite,
    ) -> Self {
        let scopes = ScopeArena::analyze(body);
        Self {
            filename: filename.to_string(),
            source,
            tokens,
            line_index: LineIndex::from_source_text(source),
            scopes,
            lowering_profile: LoweringProfile::for_suite(body),
        }
    }

    pub(super) const fn target_specific_rules_enabled(&self) -> bool {
        self.lowering_profile.target_specific_rules_enabled()
    }

    pub(super) const fn module_plan(&self) -> &ModulePlan {
        self.scopes.module()
    }
}

fn suite_has_lazy_imports(body: &Suite) -> bool {
    body.iter().any(|statement| match statement {
        Stmt::Import(import) => import.is_lazy,
        Stmt::ImportFrom(import) => import.is_lazy,
        Stmt::If(statement) => {
            suite_has_lazy_imports(&statement.body)
                || statement
                    .elif_else_clauses
                    .iter()
                    .any(|clause| suite_has_lazy_imports(&clause.body))
        }
        Stmt::For(statement) => {
            suite_has_lazy_imports(&statement.body) || suite_has_lazy_imports(&statement.orelse)
        }
        Stmt::While(statement) => {
            suite_has_lazy_imports(&statement.body) || suite_has_lazy_imports(&statement.orelse)
        }
        Stmt::With(statement) => suite_has_lazy_imports(&statement.body),
        Stmt::Match(statement) => statement
            .cases
            .iter()
            .any(|case| suite_has_lazy_imports(&case.body)),
        _ => false,
    })
}

impl Compiler<'_> {
    pub(super) fn line_number(&self, offset: u32) -> u32 {
        self.source_line(self.source_offset(offset)).number
    }

    pub(super) fn source_location(&self, range: ruff_text_size::TextRange) -> SourceLocation {
        let start = self.source_offset(u32::from(range.start()));
        let end = self.source_offset(u32::from(range.end()));
        let source_line = self.source_line(start);
        let line = i32::try_from(source_line.number).unwrap_or(i32::MAX);
        let column = i32::try_from(u32::from(start - source_line.start)).unwrap_or(i32::MAX);

        if source_line.contains(end) {
            let end_column = i32::try_from(u32::from(end - source_line.start)).unwrap_or(i32::MAX);
            return SourceLocation::new(line, line, column, end_column);
        }

        let (end_line, end_column) = self.source_position(u32::from(end));
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
        if string
            .value
            .iter()
            .all(|part| !part.as_str().contains('\u{fffd}'))
        {
            return Constant::String(string.value.to_str().to_string());
        }

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

    /// Folds constants while preserving surrogate escapes that Ruff represents as U+FFFD.
    pub(super) fn source_aware_fold_constant(&self, expression: &Expr) -> Option<Constant> {
        match expression {
            Expr::StringLiteral(string) => Some(self.string_literal_constant(string)),
            Expr::Tuple(tuple) if tuple.ctx == ExprContext::Load => tuple
                .elts
                .iter()
                .map(|element| self.source_aware_fold_constant(element))
                .collect::<Option<Vec<_>>>()
                .map(Constant::Tuple),
            Expr::BinOp(binary) if binary.op == Operator::Mult => {
                let folded = fold_constant(expression)?;
                if !matches!(&folded, Constant::String(value) if value.contains('\u{fffd}')) {
                    return Some(folded);
                }
                let left = self.source_aware_fold_constant(&binary.left)?;
                let right = self.source_aware_fold_constant(&binary.right)?;
                match (left, right) {
                    (Constant::SurrogateString(value), Constant::Int(times))
                    | (Constant::Int(times), Constant::SurrogateString(value)) => {
                        let length = value.iter().filter(|byte| **byte & 0xc0 != 0x80).count();
                        let max_times = 4096_usize.checked_div(length).unwrap_or(usize::MAX);
                        let times = usize::try_from(times).ok()?;
                        (times <= max_times).then(|| Constant::SurrogateString(value.repeat(times)))
                    }
                    _ => Some(folded),
                }
            }
            _ => fold_constant(expression),
        }
    }

    pub(super) fn generator_expression_range(
        &self,
        generator: &ruff_python_ast::ExprGenerator,
    ) -> ruff_text_size::TextRange {
        if generator.parenthesized {
            return generator.range;
        }

        ruff_python_ast::token::parentheses_iterator(generator.into(), None, self.context.tokens)
            .next()
            .unwrap_or(generator.range)
    }

    pub(super) fn debug_expression_source_range(
        &self,
        interpolation: &ruff_python_ast::InterpolatedElement,
    ) -> ruff_text_size::TextRange {
        assert!(
            interpolation.debug_text.is_some(),
            "debug expression range requires debug text"
        );
        let tokens = self.context.tokens.in_range(interpolation.range);
        let opening = tokens
            .first()
            .filter(|token| token.kind() == TokenKind::Lbrace)
            .expect("interpolation should start with an opening brace");
        let marker = tokens
            .iter()
            .find(|token| {
                token.start() >= interpolation.expression.range().end()
                    && token.kind() == TokenKind::Equal
            })
            .expect("debug interpolation should contain an equals token");
        ruff_text_size::TextRange::new(opening.end(), marker.start())
    }

    pub(super) fn fstring_result_range(
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
                    .unwrap_or_else(|| interpolated_string_content_range(fstring.into()))
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
        self.call_opcode_range_with_method(call, self.call_shape(call).direct_method)
    }

    pub(super) fn call_opcode_range_with_method(
        &self,
        call: &ruff_python_ast::ExprCall,
        direct_method: Option<&ruff_python_ast::ExprAttribute>,
    ) -> ruff_text_size::TextRange {
        if let Some(attribute) = direct_method {
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

    pub(super) fn call_shape<'a>(&self, call: &'a ruff_python_ast::ExprCall) -> CallShape<'a> {
        let positional_count = call.arguments.args.len();
        let keyword_count = call.arguments.keywords.len();
        let has_starred = call.arguments.args.iter().any(Expr::is_starred_expr);
        let has_keyword_unpack = call
            .arguments
            .keywords
            .iter()
            .any(|keyword| keyword.arg.is_none());
        let direct_method = if !has_starred
            && !has_keyword_unpack
            && positional_count + keyword_count + usize::from(keyword_count > 0) < 30
            && let Expr::Attribute(attribute) = call.func.as_ref()
            && !self.imported_module_attribute(attribute)
        {
            Some(attribute)
        } else {
            None
        };
        CallShape {
            positional_count,
            keyword_count,
            has_starred,
            has_keyword_unpack,
            direct_method,
        }
    }
}

pub(super) fn interpolated_string_content_range(
    string: ruff_python_ast::StringLike<'_>,
) -> ruff_text_size::TextRange {
    debug_assert!(string.is_interpolated_string());
    let mut parts = string.parts();
    let Some(first) = parts.next() else {
        return string.range();
    };
    let last = parts.next_back().unwrap_or(first);
    ruff_text_size::TextRange::new(first.content_range().start(), last.content_range().end())
}

pub(super) fn debug_text_range(
    interpolation: &ruff_python_ast::InterpolatedElement,
) -> ruff_text_size::TextRange {
    let debug = interpolation
        .debug_text
        .as_ref()
        .expect("debug text range requires debug text");
    ruff_text_size::TextRange::at(
        interpolation.range.start() + "{".text_len(),
        debug.as_str().text_len(),
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

/// Removes expression-level Python comments while preserving every other byte,
/// as CPython does for the source text stored by a t-string interpolation.
pub(super) fn strip_expression_comments(source: &str) -> String {
    let parsed = ruff_python_parser::parse_unchecked(
        source,
        ruff_python_parser::ParseOptions::from(ruff_python_parser::Mode::ParenthesizedExpression),
    );
    let mut output = String::with_capacity(source.len());
    let mut copied = 0;
    // Ruff tokenizes comments inside nested interpolated strings. They are part of the nested
    // string's source from the outer expression's perspective, so preserve them here.
    let mut interpolated_string_depth = 0_u32;
    for token in parsed.tokens() {
        match token.kind() {
            TokenKind::FStringStart | TokenKind::TStringStart => {
                interpolated_string_depth += 1;
            }
            TokenKind::FStringEnd | TokenKind::TStringEnd => {
                interpolated_string_depth = interpolated_string_depth.saturating_sub(1);
            }
            TokenKind::Comment if interpolated_string_depth == 0 => {
                let start = usize::from(token.start());
                let end = usize::from(token.end());
                output.push_str(&source[copied..start]);
                copied = end;
            }
            _ => {}
        }
    }
    output.push_str(&source[copied..]);
    output
}
pub(super) fn implicit_return_range(
    compiler: &Compiler<'_>,
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
            Expr::FString(fstring) => Compiler::fstring_result_range(fstring),
            expression => expression.range(),
        },
        Stmt::AnnAssign(assignment) => {
            let Expr::Subscript(subscript) = assignment.target.as_ref() else {
                return statement.range();
            };
            last_discarded_annotation_slice_range(&subscript.slice)
        }
        Stmt::While(statement) => condition_exit_range(&statement.test),
        Stmt::For(statement) => statement.iter.range(),
        _ => statement.range(),
    }
}

fn condition_exit_expression(mut expression: &Expr) -> &Expr {
    while let Expr::UnaryOp(unary) = expression
        && unary.op == ruff_python_ast::UnaryOp::Not
    {
        expression = &unary.operand;
    }
    expression
}

pub(super) fn condition_exit_range(expression: &Expr) -> ruff_text_size::TextRange {
    condition_exit_expression(expression).range()
}

pub(super) fn unary_not_condition_exit_range(
    expression: &Expr,
) -> Option<ruff_text_size::TextRange> {
    let Expr::UnaryOp(unary) = expression else {
        return None;
    };
    if unary.op != ruff_python_ast::UnaryOp::Not {
        return None;
    }

    let operand = condition_exit_expression(&unary.operand);
    if matches!(operand, Expr::BoolOp(_) | Expr::If(_)) {
        return None;
    }
    Some(operand.range())
}

pub(super) fn synthetic_condition_exit_range(expression: &Expr) -> ruff_text_size::TextRange {
    // `codegen_jump_if` strips `not`, so direct operands own synthetic exits. Boolean and
    // conditional operands choose an edge-specific child; preserve their previous whole-condition
    // owner until the caller retains enough jump-polarity information to select that child.
    unary_not_condition_exit_range(expression).unwrap_or(expression.range())
}

fn last_store_target_range(
    compiler: &Compiler<'_>,
    expression: &Expr,
) -> ruff_text_size::TextRange {
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
        Stmt::If(statement) => first_expression_instruction_range(&statement.test),
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
pub(super) fn unparse_annotation(expression: &Expr, source: &str) -> String {
    struct AnnotationStringDetector(bool);

    impl<'ast> Visitor<'ast> for AnnotationStringDetector {
        fn visit_expr(&mut self, expression: &'ast Expr) {
            if self.0 {
                return;
            }
            if matches!(expression, Expr::StringLiteral(_)) {
                self.0 = true;
            } else {
                walk_expr(self, expression);
            }
        }
    }

    struct AnnotationStringNormalizer<'source> {
        source: &'source str,
    }

    impl Transformer for AnnotationStringNormalizer<'_> {
        fn visit_expr(&self, expression: &mut Expr) {
            if let Expr::StringLiteral(string) = expression {
                let mut literal = string
                    .value
                    .iter()
                    .next()
                    .expect("string literals contain at least one part")
                    .clone();
                literal.value = string.value.to_str().into();
                literal.flags = if string.value.is_unicode()
                    && self
                        .source
                        .as_bytes()
                        .get(usize::from(literal.range.start()))
                        == Some(&b'u')
                {
                    StringLiteralFlags::empty().with_prefix(literal.flags.prefix())
                } else {
                    StringLiteralFlags::empty()
                };
                string.value = StringLiteralValue::single(literal);
            } else {
                transform_expr(self, expression);
            }
        }
    }

    if let Expr::StringLiteral(string) = expression {
        let mut annotation = python_string_repr(string.value.to_str());
        if string.value.is_unicode()
            && string.value.iter().next().is_some_and(|literal| {
                source.as_bytes().get(usize::from(literal.range.start())) == Some(&b'u')
            })
        {
            annotation.insert(0, 'u');
        }
        return annotation;
    }

    let mut detector = AnnotationStringDetector(false);
    detector.visit_expr(expression);
    let normalized = detector.0.then(|| {
        let mut expression = expression.clone();
        AnnotationStringNormalizer { source }.visit_expr(&mut expression);
        expression
    });
    let indentation = Indentation::default();
    let mut annotation = Generator::new(&indentation, LineEnding::Lf)
        .with_mode(CodegenMode::AstUnparse)
        .expr(normalized.as_ref().unwrap_or(expression));
    let parsed = ruff_python_parser::parse_unchecked(
        &annotation,
        ruff_python_parser::ParseOptions::from(ruff_python_parser::Mode::ParenthesizedExpression),
    );
    let mut stack = Vec::<(TokenKind, usize)>::new();
    let mut parenthesis_pairs = FxHashSet::default();
    let mut bracket_pairs = Vec::new();
    for token in parsed.tokens() {
        match token.kind() {
            opening @ (TokenKind::Lpar | TokenKind::Lsqb | TokenKind::Lbrace) => {
                stack.push((opening, usize::from(token.start())));
            }
            closing @ (TokenKind::Rpar | TokenKind::Rsqb | TokenKind::Rbrace) => {
                let expected = match closing {
                    TokenKind::Rpar => TokenKind::Lpar,
                    TokenKind::Rsqb => TokenKind::Lsqb,
                    TokenKind::Rbrace => TokenKind::Lbrace,
                    _ => unreachable!(),
                };
                if let Some((opening, opening_index)) = stack.pop()
                    && opening == expected
                {
                    let closing_index = usize::from(token.start());
                    if opening == TokenKind::Lpar {
                        parenthesis_pairs.insert((opening_index, closing_index));
                    } else if opening == TokenKind::Lsqb {
                        bracket_pairs.push((opening_index, closing_index));
                    }
                }
            }
            _ => {}
        }
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
    ruff_python_literal::escape::UnicodeEscape::new_repr(value)
        .str_repr(ruff_python_ast::str::TripleQuotes::No)
        .to_string()
        .expect("Python string representation length fits in usize")
}
