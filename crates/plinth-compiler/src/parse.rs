//! Parses Plinth TS with oxc and converts the result into `ast`.
//!
//! The converter rejects every syntax form outside Plinth TS v0 (SPEC.md
//! §4.3, §4.4) with a stable code and, where possible, a fix.

use crate::ast::*;
use crate::diag::{Diagnostic, FileId, Span, code};
use oxc_allocator::Allocator;
use oxc_ast::ast as o;
use oxc_parser::Parser;
use oxc_span::SourceType;

pub fn parse(file: FileId, text: &str, is_tsx: bool, diags: &mut Vec<Diagnostic>) -> Module {
    let allocator = Allocator::default();
    let source_type = if is_tsx { SourceType::tsx() } else { SourceType::ts() };
    let ret = Parser::new(&allocator, text, source_type).parse();
    for d in ret.diagnostics.iter() {
        let span = d.labels.first().map(|l| l.span()).map(|s| Span::new(file, s.start, s.end)).unwrap_or_default();
        let span = Span { file, ..span };
        let mut diag = Diagnostic::error(code::SYNTAX, span, d.message.to_string());
        if let Some(h) = &d.help {
            diag = diag.help(h.to_string());
        }
        diags.push(diag);
    }
    let mut cx = Cx { file, diags };
    let items = ret.program.body.iter().filter_map(|s| cx.item(s)).collect();
    Module { items }
}

struct Cx<'d> {
    file: FileId,
    diags: &'d mut Vec<Diagnostic>,
}

impl Cx<'_> {
    fn span(&self, s: oxc_span::Span) -> Span {
        Span::new(self.file, s.start, s.end)
    }

    fn err(&mut self, code: &'static str, s: oxc_span::Span, msg: impl Into<String>) {
        let span = self.span(s);
        self.diags.push(Diagnostic::error(code, span, msg));
    }

    fn err_help(&mut self, code: &'static str, s: oxc_span::Span, msg: impl Into<String>, help: impl Into<String>) {
        let span = self.span(s);
        self.diags.push(Diagnostic::error(code, span, msg).help(help));
    }

    // -- Items ------------------------------------------------------------

    fn item(&mut self, s: &o::Statement) -> Option<Item> {
        use o::Statement as S;
        match s {
            S::ImportDeclaration(d) => self.import(d).map(Item::Import),
            S::ExportDefaultDeclaration(d) => match &d.declaration {
                o::ExportDefaultDeclarationKind::FunctionDeclaration(f) => {
                    let mut f = self.function(f)?;
                    f.exported = true;
                    f.is_default = true;
                    Some(Item::Stmt(Stmt { span: f.span, kind: StmtKind::Func(f) }))
                }
                o::ExportDefaultDeclarationKind::ClassDeclaration(c) => {
                    self.class_err(c.span);
                    None
                }
                o::ExportDefaultDeclarationKind::TSInterfaceDeclaration(i) => {
                    self.err(code::UNSUPPORTED, i.span, "a default-exported interface is not supported");
                    None
                }
                other => {
                    let e = other.as_expression()?;
                    self.expr(e).map(Item::ExportDefault)
                }
            },
            S::ExportDeclaration(d) => self.declaration(&d.declaration, true),
            S::ExportNamedDeclaration(d) => {
                let names = d
                    .specifiers
                    .iter()
                    .map(|sp| (sp.local.name().to_string(), sp.exported.name().to_string(), self.span(sp.span)))
                    .collect();
                Some(Item::ExportNames(names))
            }
            S::ExportFromDeclaration(d) => {
                self.err_help(code::UNSUPPORTED, d.span, "re-exports are not supported yet", "import the names, then export them");
                None
            }
            S::ExportAllDeclaration(d) => {
                self.err(code::UNSUPPORTED, d.span, "`export *` is not supported");
                None
            }
            S::TSExportAssignment(d) => {
                self.err(code::UNSUPPORTED, d.span, "`export =` is not supported");
                None
            }
            S::TSNamespaceExportDeclaration(d) => {
                self.err(code::NAMESPACE, d.span, "namespaces are not supported");
                None
            }
            _ => {
                if let Some(item) = self.declaration_stmt(s) {
                    return item;
                }
                self.stmt(s).map(Item::Stmt)
            }
        }
    }

    /// Type-level declarations that can appear as statements.
    fn declaration_stmt(&mut self, s: &o::Statement) -> Option<Option<Item>> {
        use o::Statement as S;
        Some(match s {
            S::TSInterfaceDeclaration(_) | S::TSTypeAliasDeclaration(_) | S::TSEnumDeclaration(_) | S::ClassDeclaration(_) => {
                let d = s.as_declaration()?;
                self.declaration(d, false)
            }
            _ => return None,
        })
    }

    fn declaration(&mut self, d: &o::Declaration, exported: bool) -> Option<Item> {
        use o::Declaration as D;
        match d {
            D::VariableDeclaration(v) => {
                let mut decls = self.var_decl(v)?;
                for d in &mut decls {
                    d.exported = exported;
                }
                Some(Item::Stmt(Stmt { span: self.span(v.span), kind: StmtKind::Var(decls) }))
            }
            D::FunctionDeclaration(f) => {
                let mut f = self.function(f)?;
                f.exported = exported;
                Some(Item::Stmt(Stmt { span: f.span, kind: StmtKind::Func(f) }))
            }
            D::ClassDeclaration(c) => self.class_decl(c, exported).map(Item::Class),
            D::TSTypeAliasDeclaration(t) => {
                let mut type_params = Vec::new();
                if let Some(tp) = &t.type_parameters {
                    for p in &tp.params {
                        if p.constraint.is_some() || p.default.is_some() {
                            self.err_help(
                                code::GENERIC_USER,
                                p.span,
                                "a constrained or defaulted type parameter is not supported yet",
                                "use a plain type parameter: `<T>`",
                            );
                            return None;
                        }
                        type_params.push(p.name.name.to_string());
                    }
                }
                let ty = self.ty(&t.type_annotation)?;
                Some(Item::TypeAlias(TypeAlias { name: t.id.name.to_string(), ty, exported, span: self.span(t.span), type_params }))
            }
            D::TSInterfaceDeclaration(i) => {
                let mut type_params = Vec::new();
                if let Some(tp) = &i.type_parameters {
                    for p in &tp.params {
                        if p.constraint.is_some() || p.default.is_some() {
                            self.err_help(
                                code::GENERIC_USER,
                                p.span,
                                "a constrained or defaulted type parameter is not supported yet",
                                "use a plain type parameter: `<T>`",
                            );
                            return None;
                        }
                        type_params.push(p.name.name.to_string());
                    }
                }
                if !i.extends.is_empty() {
                    self.err_help(code::UNSUPPORTED, i.span, "`extends` on interfaces comes in v1", "repeat the fields");
                    return None;
                }
                let fields = self.members(&i.body.body)?;
                Some(Item::Interface(Interface { name: i.id.name.to_string(), fields, exported, span: self.span(i.span), type_params }))
            }
            D::TSEnumDeclaration(e) => {
                let mut members = Vec::new();
                for m in &e.body.members {
                    let name = match &m.id {
                        o::TSEnumMemberName::Identifier(id) => id.name.to_string(),
                        o::TSEnumMemberName::String(s) => s.value.to_string(),
                        _ => {
                            self.err(code::UNSUPPORTED, m.span, "computed enum member names are not supported");
                            continue;
                        }
                    };
                    let value = match &m.initializer {
                        None => None,
                        Some(o::Expression::NumericLiteral(n)) => Some(n.value),
                        Some(o::Expression::UnaryExpression(u))
                            if u.operator == oxc_syntax::operator::UnaryOperator::UnaryNegation =>
                        {
                            match &u.argument {
                                o::Expression::NumericLiteral(n) => Some(-n.value),
                                _ => {
                                    self.err(code::UNSUPPORTED, m.span, "enum values must be number literals");
                                    None
                                }
                            }
                        }
                        Some(_) => {
                            self.err_help(
                                code::UNSUPPORTED,
                                m.span,
                                "enum values must be number literals",
                                "for named strings, use a string literal union type",
                            );
                            None
                        }
                    };
                    members.push((name, value, self.span(m.span)));
                }
                Some(Item::Enum(EnumDecl { name: e.id.name.to_string(), members, exported, span: self.span(e.span) }))
            }
            D::TSExternalModuleDeclaration(_) | D::TSNamespaceDeclaration(_) | D::TSGlobalDeclaration(_) => {
                self.err(code::NAMESPACE, oxc_span::GetSpan::span(d), "namespaces and `declare` blocks are not allowed in app code");
                None
            }
            D::TSImportEqualsDeclaration(i) => {
                self.err(code::UNSUPPORTED, i.span, "`import =` is not supported");
                None
            }
        }
    }

    fn class_err(&mut self, s: oxc_span::Span) {
        self.err_help(
            code::CLASS,
            s,
            "this class form is not supported",
            "use an interface for the data and functions for the behavior",
        );
    }

    /// A basic class declaration (SPEC.md §4.2, v0: no `extends`). Each
    /// unsupported member gives its own diagnostic and is skipped, so the
    /// rest of the class still checks.
    fn class_decl(&mut self, c: &o::Class, exported: bool) -> Option<ClassDecl> {
        let Some(id) = &c.id else {
            self.err(code::CLASS, c.span, "a class declaration needs a name");
            return None;
        };
        if !c.implements.is_empty() {
            self.err_help(code::CLASS, c.span, "`implements` is not supported", "give the class its own fields and methods");
            return None;
        }
        let mut extends = None;
        if let Some(h) = &c.heritage {
            if h.type_arguments.is_some() {
                self.err(code::GENERIC_USER, c.span, "a generic base class is not supported yet");
                return None;
            }
            match &h.expression {
                o::Expression::Identifier(id) => extends = Some((id.name.to_string(), self.span(id.span))),
                other => {
                    self.err_help(
                        code::CLASS,
                        oxc_span::GetSpan::span(other),
                        "the base class must be a plain name",
                        "write `extends BaseClassName`",
                    );
                    return None;
                }
            }
        }
        if c.r#abstract {
            self.err(code::CLASS, c.span, "`abstract` classes are not supported");
            return None;
        }
        if c.type_parameters.is_some() {
            self.err(code::GENERIC_USER, c.span, "generic classes are not supported yet");
            return None;
        }
        let mut fields = Vec::new();
        let mut ctor = None;
        let mut methods = Vec::new();
        for el in &c.body.body {
            match el {
                o::ClassElement::PropertyDefinition(p) => {
                    if p.r#static {
                        self.err_help(code::CLASS, p.span, "static members are not supported yet", "use a top-level `let` instead");
                        continue;
                    }
                    if matches!(p.key, o::PropertyKey::PrivateIdentifier(_)) {
                        self.err_help(
                            code::CLASS,
                            p.span,
                            "`#private` fields are not supported yet",
                            "use a plain field name; v0 has no access control",
                        );
                        continue;
                    }
                    let Some(key) = p.key.static_name() else {
                        self.err(code::COMPUTED_ACCESS, p.span, "computed field names are not supported");
                        continue;
                    };
                    if p.optional {
                        self.err_help(code::CLASS, p.span, "optional class fields are not supported yet", "use `T | null` with a default of `null`");
                        continue;
                    }
                    let Some(ann) = &p.type_annotation else {
                        self.err(code::ANY, p.span, "a field needs a type");
                        continue;
                    };
                    let Some(ty) = self.ty(&ann.type_annotation) else { continue };
                    let init = match &p.value {
                        Some(e) => self.expr(e),
                        None => None,
                    };
                    fields.push(ClassField { name: key.to_string(), ty, init, span: self.span(p.span) });
                }
                o::ClassElement::MethodDefinition(m) => {
                    if m.r#static {
                        self.err_help(code::CLASS, m.span, "static members are not supported yet", "use a top-level function instead");
                        continue;
                    }
                    if matches!(m.kind, o::MethodDefinitionKind::Get | o::MethodDefinitionKind::Set) {
                        self.err_help(code::GETTER_SETTER, m.span, "getters and setters are not supported in classes yet", "use a plain method");
                        continue;
                    }
                    let f = &m.value;
                    if f.r#async || f.generator {
                        self.err(code::ASYNC, m.span, "async methods and generators come in v1");
                        continue;
                    }
                    let Some(body) = &f.body else {
                        self.err(code::NAMESPACE, m.span, "a method needs a body");
                        continue;
                    };
                    if m.kind == o::MethodDefinitionKind::Constructor {
                        if ctor.is_some() {
                            self.err(code::DUPLICATE, m.span, "a class can have only one constructor");
                            continue;
                        }
                        let Some(params) = self.ctor_params(&f.params) else { continue };
                        ctor = Some(CtorDecl { params, body: self.block(&body.statements), span: self.span(m.span) });
                        continue;
                    }
                    let Some(key) = m.key.static_name() else {
                        self.err(code::COMPUTED_ACCESS, m.span, "computed method names are not supported");
                        continue;
                    };
                    let Some(params) = self.params(&f.params) else { continue };
                    let ret = match &f.return_type {
                        Some(r) => match self.ty(&r.type_annotation) {
                            Some(t) => Some(t),
                            None => continue,
                        },
                        None => None,
                    };
                    methods.push(FuncDecl {
                        name: Some((key.to_string(), self.span(m.span))),
                        params,
                        ret,
                        body: Body::Block(self.block(&body.statements)),
                        exported: false,
                        is_default: false,
                        span: self.span(m.span),
                        type_params: Vec::new(),
                    });
                }
                o::ClassElement::AccessorProperty(a) => {
                    self.err_help(code::GETTER_SETTER, a.span, "accessor properties are not supported", "use a plain field and method");
                }
                o::ClassElement::StaticBlock(s) => {
                    self.err(code::CLASS, s.span, "static blocks are not supported");
                }
                o::ClassElement::TSIndexSignature(s) => {
                    self.err(code::COMPUTED_ACCESS, s.span, "index signatures are not supported in classes");
                }
            }
        }
        Some(ClassDecl { name: id.name.to_string(), fields, ctor, methods, exported, span: self.span(c.span), extends })
    }

    /// Constructor parameters: like `params`, but parameter properties
    /// (`constructor(public x: number)`) are not supported.
    fn ctor_params(&mut self, params: &o::FormalParameters) -> Option<Vec<Param>> {
        for p in &params.items {
            if p.accessibility.is_some() || p.readonly || p.r#override {
                self.err_help(
                    code::CLASS,
                    p.span,
                    "parameter properties are not supported yet",
                    "declare the field on the class and assign it in the constructor body",
                );
                return None;
            }
        }
        self.params(params)
    }

    fn import(&mut self, d: &o::ImportDeclaration) -> Option<Import> {
        let mut imp = Import {
            source: d.source.value.to_string(),
            source_span: self.span(d.source.span),
            default: None,
            names: Vec::new(),
            type_only: d.import_kind.is_type(),
            span: self.span(d.span),
        };
        for sp in d.specifiers.iter().flatten() {
            match sp {
                o::ImportDeclarationSpecifier::ImportSpecifier(s) => {
                    if s.import_kind.is_type() {
                        continue;
                    }
                    imp.names.push((s.imported.name().to_string(), s.local.name.to_string(), self.span(s.span)));
                }
                o::ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => {
                    imp.default = Some((s.local.name.to_string(), self.span(s.span)));
                }
                o::ImportDeclarationSpecifier::ImportNamespaceSpecifier(s) => {
                    self.err_help(code::UNSUPPORTED, s.span, "namespace imports are not supported", "import the names you use");
                }
            }
        }
        Some(imp)
    }

    fn members(&mut self, sigs: &[o::TSSignature]) -> Option<Vec<FieldAnn>> {
        let mut fields = Vec::new();
        for m in sigs {
            match m {
                o::TSSignature::TSPropertySignature(p) => {
                    let Some(name) = p.key.static_name() else {
                        self.err(code::COMPUTED_ACCESS, p.span, "computed property names are not supported");
                        continue;
                    };
                    let Some(ann) = &p.type_annotation else {
                        self.err(code::ANY, p.span, "a field needs a type");
                        continue;
                    };
                    let ty = self.ty(&ann.type_annotation)?;
                    fields.push(FieldAnn { name: name.to_string(), ty, optional: p.optional, span: self.span(p.span) });
                }
                o::TSSignature::TSMethodSignature(m) => {
                    let Some(name) = m.key.static_name() else {
                        self.err(code::COMPUTED_ACCESS, m.span, "computed property names are not supported");
                        continue;
                    };
                    let params = self.type_params(&m.params)?;
                    let ret = match &m.return_type {
                        Some(r) => self.ty(&r.type_annotation)?,
                        None => TypeAnn::Void(self.span(m.span)),
                    };
                    let span = self.span(m.span);
                    fields.push(FieldAnn {
                        name: name.to_string(),
                        ty: TypeAnn::Func { params, ret: Box::new(ret), span },
                        optional: m.optional,
                        span,
                    });
                }
                o::TSSignature::TSIndexSignature(s) => {
                    self.err_help(code::COMPUTED_ACCESS, s.span, "index signatures are not supported", "use a Map");
                }
                o::TSSignature::TSCallSignatureDeclaration(s) => {
                    self.err(code::UNSUPPORTED, s.span, "call signatures are not supported");
                }
                o::TSSignature::TSConstructSignatureDeclaration(s) => {
                    self.err(code::CLASS, s.span, "construct signatures are not supported");
                }
            }
        }
        Some(fields)
    }

    // -- Types ------------------------------------------------------------

    fn ty(&mut self, t: &o::TSType) -> Option<TypeAnn> {
        use o::TSType as T;
        let span = self.span(oxc_span::GetSpan::span(t));
        Some(match t {
            T::TSNumberKeyword(_) => TypeAnn::Number(span),
            T::TSStringKeyword(_) => TypeAnn::String(span),
            T::TSBooleanKeyword(_) => TypeAnn::Boolean(span),
            T::TSVoidKeyword(_) => TypeAnn::Void(span),
            T::TSNullKeyword(_) | T::TSUndefinedKeyword(_) => TypeAnn::Null(span),
            T::TSAnyKeyword(_) | T::TSUnknownKeyword(_) => {
                self.err_help(code::ANY, t_span(t), "`any` and `unknown` are not allowed", "write the real type");
                return None;
            }
            T::TSParenthesizedType(p) => return self.ty(&p.type_annotation),
            T::TSArrayType(a) => TypeAnn::Array(Box::new(self.ty(&a.element_type)?), span),
            T::TSTypeOperatorType(op) if op.operator == o::TSTypeOperatorOperator::Readonly => {
                return self.ty(&op.type_annotation);
            }
            T::TSUnionType(u) => {
                let mut parts = Vec::new();
                for p in &u.types {
                    parts.push(self.ty(p)?);
                }
                TypeAnn::Union(parts, span)
            }
            T::TSLiteralType(l) => match &l.literal {
                o::TSLiteral::StringLiteral(s) => TypeAnn::StrLit(s.value.to_string(), span),
                o::TSLiteral::NumericLiteral(n) => TypeAnn::NumLit(n.value, span),
                o::TSLiteral::BooleanLiteral(b) => TypeAnn::BoolLit(b.value, span),
                _ => {
                    self.err_help(
                        code::ADVANCED_TYPE,
                        t_span(t),
                        "this literal type is not supported",
                        "use `string`, `number` or `boolean` literal types",
                    );
                    return None;
                }
            },
            T::TSTypeReference(r) => {
                let name = match &r.type_name {
                    o::TSTypeName::IdentifierReference(id) => id.name.to_string(),
                    _ => {
                        self.err(code::NAMESPACE, r.span, "qualified type names are not supported");
                        return None;
                    }
                };
                let mut args = Vec::new();
                if let Some(ta) = &r.type_arguments {
                    for p in &ta.params {
                        args.push(self.ty(p)?);
                    }
                }
                TypeAnn::Named { name, args, span }
            }
            T::TSFunctionType(f) => {
                if f.type_parameters.is_some() {
                    self.err(code::GENERIC_USER, f.span, "generic function types come in v1");
                    return None;
                }
                let params = self.type_params(&f.params)?;
                let ret = self.ty(&f.return_type.type_annotation)?;
                TypeAnn::Func { params, ret: Box::new(ret), span }
            }
            T::TSTypeLiteral(l) => TypeAnn::Object(self.members(&l.members)?, span),
            T::TSConditionalType(_)
            | T::TSMappedType(_)
            | T::TSIndexedAccessType(_)
            | T::TSInferType(_)
            | T::TSTemplateLiteralType(_)
            | T::TSTypeQuery(_)
            | T::TSTypeOperatorType(_)
            | T::TSIntersectionType(_)
            | T::TSTupleType(_)
            | T::TSImportType(_)
            | T::TSTypePredicate(_) => {
                self.err_help(
                    code::ADVANCED_TYPE,
                    t_span(t),
                    "this type form is not supported in Plinth TS",
                    "write a concrete type: an interface, an array, or a union with null",
                );
                return None;
            }
            _ => {
                self.err(code::ADVANCED_TYPE, t_span(t), "this type is not supported");
                return None;
            }
        })
    }

    fn type_params(&mut self, params: &o::FormalParameters) -> Option<Vec<(String, TypeAnn, bool)>> {
        if params.rest.is_some() {
            self.err(code::UNSUPPORTED, params.span, "rest parameters are not supported");
            return None;
        }
        let mut out = Vec::new();
        for p in &params.items {
            let name = match &p.pattern {
                o::BindingPattern::BindingIdentifier(id) => id.name.to_string(),
                _ => "_".into(),
            };
            let Some(ann) = &p.type_annotation else {
                self.err(code::ANY, p.span, "a parameter needs a type");
                return None;
            };
            out.push((name, self.ty(&ann.type_annotation)?, p.optional));
        }
        Some(out)
    }

    // -- Statements -------------------------------------------------------

    fn var_decl(&mut self, v: &o::VariableDeclaration) -> Option<Vec<VarDecl>> {
        let kind = match v.kind {
            o::VariableDeclarationKind::Const => VarKind::Const,
            o::VariableDeclarationKind::Let => VarKind::Let,
            o::VariableDeclarationKind::Var => {
                self.err_help(code::VAR, v.span, "`var` is not allowed", "use `let` or `const`");
                VarKind::Let
            }
            _ => {
                self.err(code::UNSUPPORTED, v.span, "`using` declarations are not supported");
                return None;
            }
        };
        if v.declare {
            self.err(code::NAMESPACE, v.span, "`declare` is not allowed in app code");
            return None;
        }
        let mut out = Vec::new();
        for d in &v.declarations {
            let pattern = self.pattern(&d.id)?;
            let ty = match &d.type_annotation {
                Some(a) => Some(self.ty(&a.type_annotation)?),
                None => None,
            };
            let init = match &d.init {
                Some(e) => Some(self.expr(e)?),
                None => None,
            };
            out.push(VarDecl { kind, pattern, ty, init, exported: false, span: self.span(d.span) });
        }
        Some(out)
    }

    fn pattern(&mut self, p: &o::BindingPattern) -> Option<Pattern> {
        match p {
            o::BindingPattern::BindingIdentifier(id) => Some(Pattern::Ident(id.name.to_string(), self.span(id.span))),
            o::BindingPattern::ObjectPattern(op) => {
                if op.rest.is_some() {
                    self.err(code::UNSUPPORTED, op.span, "rest elements in patterns are not supported");
                    return None;
                }
                let mut props = Vec::new();
                for prop in &op.properties {
                    let Some(key) = prop.key.static_name() else {
                        self.err(code::COMPUTED_ACCESS, prop.span, "computed keys in patterns are not supported");
                        return None;
                    };
                    props.push((key.to_string(), self.pattern(&prop.value)?));
                }
                Some(Pattern::Object(props, self.span(op.span)))
            }
            o::BindingPattern::ArrayPattern(ap) => {
                if ap.rest.is_some() {
                    self.err(code::UNSUPPORTED, ap.span, "rest elements in patterns are not supported");
                    return None;
                }
                let mut elems = Vec::new();
                for e in &ap.elements {
                    elems.push(match e {
                        Some(p) => Some(self.pattern(p)?),
                        None => None,
                    });
                }
                Some(Pattern::Array(elems, self.span(ap.span)))
            }
            o::BindingPattern::AssignmentPattern(a) => {
                self.err(code::UNSUPPORTED, a.span, "default values in patterns are not supported");
                None
            }
        }
    }

    fn block(&mut self, stmts: &[o::Statement]) -> Vec<Stmt> {
        stmts.iter().filter_map(|s| self.stmt(s)).collect()
    }

    fn stmt(&mut self, s: &o::Statement) -> Option<Stmt> {
        use o::Statement as S;
        let span = self.span(oxc_span::GetSpan::span(s));
        let kind = match s {
            S::BlockStatement(b) => StmtKind::Block(self.block(&b.body)),
            S::EmptyStatement(_) => StmtKind::Empty,
            S::ExpressionStatement(e) => StmtKind::Expr(self.expr(&e.expression)?),
            S::VariableDeclaration(v) => StmtKind::Var(self.var_decl(v)?),
            S::FunctionDeclaration(f) => StmtKind::Func(self.function(f)?),
            S::IfStatement(i) => StmtKind::If(
                self.expr(&i.test)?,
                Box::new(self.stmt(&i.consequent)?),
                match &i.alternate {
                    Some(a) => Some(Box::new(self.stmt(a)?)),
                    None => None,
                },
            ),
            S::WhileStatement(w) => StmtKind::While(self.expr(&w.test)?, Box::new(self.stmt(&w.body)?)),
            S::DoWhileStatement(w) => StmtKind::DoWhile(Box::new(self.stmt(&w.body)?), self.expr(&w.test)?),
            S::ForStatement(f) => {
                let init = match &f.init {
                    None => None,
                    Some(o::ForStatementInit::VariableDeclaration(v)) => {
                        Some(Box::new(Stmt { span: self.span(v.span), kind: StmtKind::Var(self.var_decl(v)?) }))
                    }
                    Some(other) => {
                        let e = other.as_expression()?;
                        let ex = self.expr(e)?;
                        Some(Box::new(Stmt { span: ex.span, kind: StmtKind::Expr(ex) }))
                    }
                };
                let test = match &f.test {
                    Some(t) => Some(self.expr(t)?),
                    None => None,
                };
                let update = match &f.update {
                    Some(u) => Some(self.expr(u)?),
                    None => None,
                };
                StmtKind::For { init, test, update, body: Box::new(self.stmt(&f.body)?) }
            }
            S::ForOfStatement(f) => {
                if f.r#await {
                    self.err(code::ASYNC, f.span, "`for await` is not supported");
                    return None;
                }
                let (kind, pattern) = match &f.left {
                    o::ForStatementLeft::VariableDeclaration(v) if v.declarations.len() == 1 => {
                        let kind = if v.kind == o::VariableDeclarationKind::Const { VarKind::Const } else { VarKind::Let };
                        (kind, self.pattern(&v.declarations[0].id)?)
                    }
                    _ => {
                        self.err_help(code::UNSUPPORTED, f.span, "the `for…of` variable must be a new declaration", "write `for (const x of xs)`");
                        return None;
                    }
                };
                StmtKind::ForOf { kind, pattern, iter: self.expr(&f.right)?, body: Box::new(self.stmt(&f.body)?) }
            }
            S::ForInStatement(f) => {
                self.err_help(code::FOR_IN, f.span, "`for…in` is not supported", "use a Map and `for…of`");
                return None;
            }
            S::ReturnStatement(r) => StmtKind::Return(match &r.argument {
                Some(a) => Some(self.expr(a)?),
                None => None,
            }),
            S::BreakStatement(b) => {
                if b.label.is_some() {
                    self.err(code::LABEL, b.span, "labels are not supported");
                }
                StmtKind::Break
            }
            S::ContinueStatement(c) => {
                if c.label.is_some() {
                    self.err(code::LABEL, c.span, "labels are not supported");
                }
                StmtKind::Continue
            }
            S::SwitchStatement(sw) => {
                let disc = self.expr(&sw.discriminant)?;
                let mut cases = Vec::new();
                for c in &sw.cases {
                    let test = match &c.test {
                        Some(t) => Some(self.expr(t)?),
                        None => None,
                    };
                    cases.push((test, self.block(&c.consequent)));
                }
                StmtKind::Switch(disc, cases)
            }
            S::ThrowStatement(t) => StmtKind::Throw(self.expr(&t.argument)?),
            S::TryStatement(t) => {
                self.err_help(
                    code::TRY,
                    t.span,
                    "`try`/`catch` comes in v1",
                    "return a `Result` for recoverable errors; `throw` stops the app",
                );
                return None;
            }
            S::LabeledStatement(l) => {
                self.err(code::LABEL, l.span, "labels are not supported");
                return None;
            }
            S::WithStatement(w) => {
                self.err(code::UNSUPPORTED, w.span, "`with` is not allowed");
                return None;
            }
            S::DebuggerStatement(d) => {
                self.err(code::UNSUPPORTED, d.span, "`debugger` is not supported");
                return None;
            }
            S::ClassDeclaration(c) => {
                self.class_err(c.span);
                return None;
            }
            S::ImportDeclaration(d) => {
                self.err(code::UNSUPPORTED, d.span, "imports must be at the top level");
                return None;
            }
            _ => {
                if let Some(d) = s.as_declaration() {
                    return match self.declaration(d, false)? {
                        Item::Stmt(st) => Some(st),
                        _ => {
                            self.err(code::UNSUPPORTED, oxc_span::GetSpan::span(s), "type declarations must be at the top level");
                            None
                        }
                    };
                }
                self.err(code::UNSUPPORTED, oxc_span::GetSpan::span(s), "this statement is not supported");
                return None;
            }
        };
        Some(Stmt { kind, span })
    }

    fn function(&mut self, f: &o::Function) -> Option<FuncDecl> {
        if f.r#async || f.generator {
            self.err(code::ASYNC, f.span, "async functions and generators come in v1");
            return None;
        }
        let mut type_params = Vec::new();
        if let Some(tp) = &f.type_parameters {
            for p in &tp.params {
                if p.constraint.is_some() || p.default.is_some() {
                    self.err_help(
                        code::GENERIC_USER,
                        p.span,
                        "a constrained or defaulted type parameter is not supported yet",
                        "use a plain type parameter: `<T>`",
                    );
                    return None;
                }
                type_params.push(p.name.name.to_string());
            }
        }
        if f.this_param.is_some() {
            self.err(code::THIS, f.span, "`this` parameters are not allowed");
            return None;
        }
        let Some(body) = &f.body else {
            self.err(code::NAMESPACE, f.span, "a function needs a body");
            return None;
        };
        Some(FuncDecl {
            name: f.id.as_ref().map(|id| (id.name.to_string(), self.span(id.span))),
            params: self.params(&f.params)?,
            ret: match &f.return_type {
                Some(r) => Some(self.ty(&r.type_annotation)?),
                None => None,
            },
            body: Body::Block(self.block(&body.statements)),
            exported: false,
            is_default: false,
            span: self.span(f.span),
            type_params,
        })
    }

    fn params(&mut self, params: &o::FormalParameters) -> Option<Vec<Param>> {
        if let Some(r) = &params.rest {
            self.err(code::UNSUPPORTED, r.span, "rest parameters are not supported");
            return None;
        }
        let mut out = Vec::new();
        for p in &params.items {
            let ty = match &p.type_annotation {
                Some(a) => Some(self.ty(&a.type_annotation)?),
                None => None,
            };
            let default = match &p.initializer {
                Some(e) => Some(self.expr(e)?),
                None => None,
            };
            out.push(Param { pattern: self.pattern(&p.pattern)?, ty, default, optional: p.optional, span: self.span(p.span) });
        }
        Some(out)
    }

    // -- Expressions ------------------------------------------------------

    fn expr(&mut self, e: &o::Expression) -> Option<Expr> {
        use o::Expression as E;
        use oxc_syntax::operator::{AssignmentOperator as A, BinaryOperator as B, LogicalOperator as L, UnaryOperator as U};
        let span = self.span(oxc_span::GetSpan::span(e));
        let kind = match e {
            E::NumericLiteral(n) => ExprKind::Num(n.value),
            E::StringLiteral(s) => ExprKind::Str(s.value.to_string()),
            E::BooleanLiteral(b) => ExprKind::Bool(b.value),
            E::NullLiteral(_) => ExprKind::Null,
            E::Identifier(id) if id.name == "undefined" => ExprKind::Null,
            E::Identifier(id) => ExprKind::Ident(id.name.to_string()),
            // `super` is a reserved word, so it can never collide with a
            // real binding; the checker recognizes `super(...)` and
            // `super.m(...)` by this sentinel name (SPEC.md §4.2 v1).
            E::Super(_) => ExprKind::Ident("super".to_string()),
            E::TemplateLiteral(t) => {
                let quasis = t.quasis.iter().map(|q| q.value.cooked.as_ref().map(|c| c.to_string()).unwrap_or_default()).collect();
                let mut exprs = Vec::new();
                for x in &t.expressions {
                    exprs.push(self.expr(x)?);
                }
                ExprKind::Template(quasis, exprs)
            }
            E::ParenthesizedExpression(p) => return self.expr(&p.expression),
            E::ArrayExpression(a) => {
                let mut elems = Vec::new();
                for el in &a.elements {
                    match el {
                        o::ArrayExpressionElement::SpreadElement(s) => elems.push((true, self.expr(&s.argument)?)),
                        o::ArrayExpressionElement::Elision(el) => {
                            self.err(code::UNSUPPORTED, el.span, "array holes are not supported");
                            return None;
                        }
                        other => elems.push((false, self.expr(other.as_expression()?)?)),
                    }
                }
                ExprKind::Array(elems)
            }
            E::ObjectExpression(obj) => {
                let mut props = Vec::new();
                for p in &obj.properties {
                    match p {
                        o::ObjectPropertyKind::SpreadProperty(s) => props.push(ObjProp::Spread(self.expr(&s.argument)?)),
                        o::ObjectPropertyKind::ObjectProperty(p) => {
                            if p.kind != o::PropertyKind::Init {
                                self.err(code::GETTER_SETTER, p.span, "getters and setters are not supported");
                                return None;
                            }
                            if p.computed {
                                self.err(code::COMPUTED_ACCESS, p.span, "computed property names are not supported");
                                return None;
                            }
                            let Some(key) = p.key.static_name() else {
                                self.err(code::COMPUTED_ACCESS, p.span, "this property name is not supported");
                                return None;
                            };
                            props.push(ObjProp::Field(key.to_string(), self.expr(&p.value)?, self.span(p.span)));
                        }
                    }
                }
                ExprKind::Object(props)
            }
            E::StaticMemberExpression(m) => ExprKind::Member {
                obj: Box::new(self.expr(&m.object)?),
                prop: m.property.name.to_string(),
                prop_span: self.span(m.property.span),
                optional: m.optional,
            },
            E::ComputedMemberExpression(m) => ExprKind::Index {
                obj: Box::new(self.expr(&m.object)?),
                index: Box::new(self.expr(&m.expression)?),
                optional: m.optional,
            },
            E::PrivateFieldExpression(p) => {
                self.class_err(p.span);
                return None;
            }
            E::ChainExpression(c) => {
                return match &c.expression {
                    o::ChainElement::CallExpression(call) => self.call(call, span),
                    o::ChainElement::TSNonNullExpression(n) => {
                        self.non_null_err(n.span);
                        None
                    }
                    other => {
                        let m = other.as_member_expression()?;
                        self.member(m, span)
                    }
                };
            }
            E::CallExpression(c) => return self.call(c, span),
            E::UnaryExpression(u) => {
                let op = match u.operator {
                    U::UnaryNegation => UnOp::Neg,
                    U::UnaryPlus => UnOp::Plus,
                    U::LogicalNot => UnOp::Not,
                    U::Delete => {
                        self.err_help(code::DELETE, u.span, "`delete` is not allowed", "use a Map and `map.delete(key)`");
                        return None;
                    }
                    U::Typeof => UnOp::Typeof,
                    _ => {
                        self.err(code::UNSUPPORTED, u.span, "this operator is not supported");
                        return None;
                    }
                };
                ExprKind::Unary(op, Box::new(self.expr(&u.argument)?))
            }
            E::BinaryExpression(b) => {
                let op = match b.operator {
                    B::Addition => BinOp::Add,
                    B::Subtraction => BinOp::Sub,
                    B::Multiplication => BinOp::Mul,
                    B::Division => BinOp::Div,
                    B::Remainder => BinOp::Rem,
                    B::Exponential => BinOp::Pow,
                    B::StrictEquality => BinOp::Eq,
                    B::StrictInequality => BinOp::Ne,
                    B::LessThan => BinOp::Lt,
                    B::LessEqualThan => BinOp::Le,
                    B::GreaterThan => BinOp::Gt,
                    B::GreaterEqualThan => BinOp::Ge,
                    B::Equality | B::Inequality => {
                        let fix = if b.operator == B::Equality { "===" } else { "!==" };
                        self.err_help(code::LOOSE_EQUALITY, b.span, "loose equality is not allowed", format!("use `{fix}`"));
                        return None;
                    }
                    B::Instanceof => {
                        let o::Expression::Identifier(id) = &b.right else {
                            self.err_help(
                                code::CLASS,
                                b.span,
                                "the right side of `instanceof` must be a class name",
                                "write `x instanceof ClassName`",
                            );
                            return None;
                        };
                        return Some(Expr {
                            kind: ExprKind::InstanceOf(Box::new(self.expr(&b.left)?), id.name.to_string(), self.span(id.span)),
                            span,
                        });
                    }
                    B::In => {
                        self.err(code::UNSUPPORTED, b.span, "`in` is not supported");
                        return None;
                    }
                    _ => {
                        self.err(code::UNSUPPORTED, b.span, "bitwise operators come with `int`");
                        return None;
                    }
                };
                ExprKind::Binary(op, Box::new(self.expr(&b.left)?), Box::new(self.expr(&b.right)?))
            }
            E::LogicalExpression(l) => {
                let op = match l.operator {
                    L::And => LogicOp::And,
                    L::Or => LogicOp::Or,
                    L::Coalesce => LogicOp::Nullish,
                };
                ExprKind::Logical(op, Box::new(self.expr(&l.left)?), Box::new(self.expr(&l.right)?))
            }
            E::ConditionalExpression(c) => ExprKind::Cond(
                Box::new(self.expr(&c.test)?),
                Box::new(self.expr(&c.consequent)?),
                Box::new(self.expr(&c.alternate)?),
            ),
            E::AssignmentExpression(a) => {
                let op = match a.operator {
                    A::Assign => None,
                    A::Addition => Some(BinOp::Add),
                    A::Subtraction => Some(BinOp::Sub),
                    A::Multiplication => Some(BinOp::Mul),
                    A::Division => Some(BinOp::Div),
                    A::Remainder => Some(BinOp::Rem),
                    _ => {
                        self.err(code::UNSUPPORTED, a.span, "this assignment operator is not supported");
                        return None;
                    }
                };
                let target = match a.left.as_simple_assignment_target() {
                    Some(t) => self.simple_target(t)?,
                    None => {
                        self.err(code::UNSUPPORTED, a.span, "destructuring assignment is not supported; use a declaration");
                        return None;
                    }
                };
                ExprKind::Assign { op, target: Box::new(target), value: Box::new(self.expr(&a.right)?) }
            }
            E::UpdateExpression(u) => ExprKind::Update {
                inc: u.operator == oxc_syntax::operator::UpdateOperator::Increment,
                prefix: u.prefix,
                target: Box::new(self.simple_target(&u.argument)?),
            },
            E::ArrowFunctionExpression(a) => {
                if a.r#async {
                    self.err(code::ASYNC, a.span, "async functions come in v1");
                    return None;
                }
                if a.type_parameters.is_some() {
                    self.err(code::GENERIC_USER, a.span, "generic functions come in v1");
                    return None;
                }
                let body = match &a.body {
                    o::ArrowFunctionBody::FunctionBody(b) => Body::Block(self.block(&b.statements)),
                    other => Body::Expr(Box::new(self.expr(other.as_expression()?)?)),
                };
                // oxc gives `(x) => expr` a FunctionBody with one expression
                // statement when `expression` is set; normalize it.
                let body = normalize_arrow_body(body, a.is_expression());
                ExprKind::Func(Box::new(FuncDecl {
                    name: None,
                    params: self.params(&a.params)?,
                    ret: match &a.return_type {
                        Some(r) => Some(self.ty(&r.type_annotation)?),
                        None => None,
                    },
                    body,
                    exported: false,
                    is_default: false,
                    span,
                    type_params: Vec::new(),
                }))
            }
            E::FunctionExpression(f) => ExprKind::Func(Box::new(self.function(f)?)),
            E::JSXElement(j) => ExprKind::Jsx(Box::new(self.jsx(j)?)),
            E::JSXFragment(f) => {
                self.err_help(code::BAD_CHILD, f.span, "fragments are not supported", "use a <Section> or <Group>");
                return None;
            }
            E::TSAsExpression(a) => {
                self.err_help(code::TYPE_ASSERTION, a.span, "`as` casts are not allowed", "annotate the variable type instead");
                return None;
            }
            E::TSSatisfiesExpression(s) => return self.expr(&s.expression),
            E::TSTypeAssertion(a) => {
                self.err(code::TYPE_ASSERTION, a.span, "type assertions are not allowed");
                return None;
            }
            E::TSNonNullExpression(n) => {
                self.non_null_err(n.span);
                return None;
            }
            // `this` is only valid inside a method or constructor body; the
            // checker reports `code::THIS` when it is not bound there.
            E::ThisExpression(_) => ExprKind::Ident("this".to_string()),
            E::ClassExpression(c) => {
                self.class_err(c.span);
                return None;
            }
            E::NewExpression(n) => {
                let name = match &n.callee {
                    o::Expression::Identifier(id) => id.name.to_string(),
                    _ => {
                        self.err_help(code::CLASS, n.span, "this `new` expression is not supported", "`new` only works on a class name");
                        return None;
                    }
                };
                if name == "Map" || name == "Set" {
                    if !n.arguments.is_empty() {
                        self.err(code::ARG_COUNT, n.span, format!("`new {name}()` takes no arguments"));
                        return None;
                    }
                    let mut type_args = Vec::new();
                    if let Some(a) = &n.type_arguments {
                        for t in &a.params {
                            type_args.push(self.ty(t)?);
                        }
                    }
                    ExprKind::New(name, type_args)
                } else {
                    if n.type_arguments.is_some() {
                        self.err(code::GENERIC_USER, n.span, "generic classes are not supported yet");
                        return None;
                    }
                    let mut args = Vec::new();
                    for a in &n.arguments {
                        match a {
                            o::Argument::SpreadElement(s) => {
                                self.err(code::UNSUPPORTED, s.span, "spread arguments are not supported");
                                return None;
                            }
                            other => args.push(self.expr(other.as_expression()?)?),
                        }
                    }
                    ExprKind::NewInstance(name, args)
                }
            }
            E::AwaitExpression(a) => {
                self.err(code::ASYNC, a.span, "`await` comes in v1");
                return None;
            }
            E::RegExpLiteral(r) => {
                self.err(code::REGEX, r.span, "regular expressions are not supported");
                return None;
            }
            E::BigIntLiteral(b) => {
                self.err(code::BIGINT, b.span, "BigInt is not supported");
                return None;
            }
            E::SequenceExpression(s) => {
                self.err(code::UNSUPPORTED, s.span, "the comma operator is not supported");
                return None;
            }
            E::TaggedTemplateExpression(t) => {
                self.err(code::UNSUPPORTED, t.span, "tagged templates are not supported");
                return None;
            }
            _ => {
                self.err(code::UNSUPPORTED, oxc_span::GetSpan::span(e), "this expression is not supported");
                return None;
            }
        };
        Some(Expr { kind, span })
    }

    fn non_null_err(&mut self, s: oxc_span::Span) {
        self.err_help(
            code::NON_NULL_ASSERTION,
            s,
            "`!` non-null assertions are not allowed",
            "check for null first (`if (x !== null)`) or use `??`",
        );
    }

    fn member(&mut self, m: &o::MemberExpression, span: Span) -> Option<Expr> {
        let kind = match m {
            o::MemberExpression::StaticMemberExpression(m) => ExprKind::Member {
                obj: Box::new(self.expr(&m.object)?),
                prop: m.property.name.to_string(),
                prop_span: self.span(m.property.span),
                optional: m.optional,
            },
            o::MemberExpression::ComputedMemberExpression(m) => ExprKind::Index {
                obj: Box::new(self.expr(&m.object)?),
                index: Box::new(self.expr(&m.expression)?),
                optional: m.optional,
            },
            o::MemberExpression::PrivateFieldExpression(p) => {
                self.class_err(p.span);
                return None;
            }
        };
        Some(Expr { kind, span })
    }

    fn simple_target(&mut self, t: &o::SimpleAssignmentTarget) -> Option<Expr> {
        let span = self.span(oxc_span::GetSpan::span(t));
        match t {
            o::SimpleAssignmentTarget::AssignmentTargetIdentifier(id) => {
                Some(Expr { kind: ExprKind::Ident(id.name.to_string()), span })
            }
            o::SimpleAssignmentTarget::TSNonNullExpression(n) => {
                self.non_null_err(n.span);
                None
            }
            o::SimpleAssignmentTarget::TSAsExpression(_)
            | o::SimpleAssignmentTarget::TSSatisfiesExpression(_)
            | o::SimpleAssignmentTarget::TSTypeAssertion(_) => {
                self.err(code::TYPE_ASSERTION, span_of(t), "type assertions are not allowed");
                None
            }
            other => {
                let m = other.as_member_expression()?;
                self.member(m, span)
            }
        }
    }

    fn call(&mut self, c: &o::CallExpression, span: Span) -> Option<Expr> {
        let mut type_args = Vec::new();
        if let Some(ta) = &c.type_arguments {
            for p in &ta.params {
                type_args.push(self.ty(p)?);
            }
        }
        let mut args = Vec::new();
        for a in &c.arguments {
            match a {
                o::Argument::SpreadElement(s) => {
                    self.err(code::UNSUPPORTED, s.span, "spread arguments are not supported");
                    return None;
                }
                other => args.push(self.expr(other.as_expression()?)?),
            }
        }
        if let o::Expression::Identifier(id) = &c.callee
            && (id.name == "eval" || id.name == "Function")
        {
            self.err(code::UNSUPPORTED, c.span, "`eval` and `Function` are not allowed");
            return None;
        }
        Some(Expr {
            kind: ExprKind::Call { callee: Box::new(self.expr(&c.callee)?), type_args, args, optional: c.optional },
            span,
        })
    }

    // -- JSX --------------------------------------------------------------

    fn jsx(&mut self, j: &o::JSXElement) -> Option<JsxElement> {
        let open = &j.opening_element;
        let name = match &open.name {
            o::JSXElementName::Identifier(id) => id.name.to_string(),
            o::JSXElementName::IdentifierReference(id) => id.name.to_string(),
            _ => {
                self.err(code::UNKNOWN_CONTROL, open.span, "this element name is not supported");
                return None;
            }
        };
        let mut attrs = Vec::new();
        for a in &open.attributes {
            match a {
                o::JSXAttributeItem::SpreadAttribute(s) => {
                    self.err(code::UNSUPPORTED, s.span, "spread attributes are not supported");
                    return None;
                }
                o::JSXAttributeItem::Attribute(attr) => {
                    let name = match &attr.name {
                        o::JSXAttributeName::Identifier(id) => id.name.to_string(),
                        o::JSXAttributeName::NamespacedName(n) => {
                            self.err(code::UNKNOWN_PROP, n.span, "namespaced attributes are not supported");
                            return None;
                        }
                    };
                    let value = match &attr.value {
                        None => None,
                        Some(o::JSXAttributeValue::StringLiteral(s)) => {
                            Some(Expr { kind: ExprKind::Str(s.value.to_string()), span: self.span(s.span) })
                        }
                        Some(o::JSXAttributeValue::ExpressionContainer(c)) => match c.expression.as_expression() {
                            Some(e) => Some(self.expr(e)?),
                            None => {
                                self.err(code::UNKNOWN_PROP, c.span, "an attribute needs a value");
                                return None;
                            }
                        },
                        Some(o::JSXAttributeValue::Element(e)) => {
                            let el = self.jsx(e)?;
                            Some(Expr { span: el.span, kind: ExprKind::Jsx(Box::new(el)) })
                        }
                        Some(o::JSXAttributeValue::Fragment(f)) => {
                            self.err(code::BAD_CHILD, f.span, "fragments are not supported");
                            return None;
                        }
                    };
                    attrs.push(JsxAttr { name, value, span: self.span(attr.span) });
                }
            }
        }
        let mut children = Vec::new();
        for c in &j.children {
            match c {
                o::JSXChild::Text(t) => {
                    if let Some(text) = jsx_text(&t.value) {
                        children.push(JsxChild::Text(text, self.span(t.span)));
                    }
                }
                o::JSXChild::Element(e) => children.push(JsxChild::Element(self.jsx(e)?)),
                o::JSXChild::ExpressionContainer(ec) => {
                    if let Some(e) = ec.expression.as_expression() {
                        children.push(JsxChild::Expr(self.expr(e)?));
                    }
                }
                o::JSXChild::Fragment(f) => {
                    self.err(code::BAD_CHILD, f.span, "fragments are not supported");
                    return None;
                }
                o::JSXChild::Spread(s) => {
                    self.err(code::BAD_CHILD, s.span, "spread children are not supported");
                    return None;
                }
            }
        }
        Some(JsxElement { name, name_span: self.span(open.span), attrs, children, span: self.span(j.span) })
    }
}

fn normalize_arrow_body(body: Body, expression: bool) -> Body {
    if !expression {
        return body;
    }
    match body {
        Body::Block(mut stmts) if stmts.len() == 1 => match stmts.pop().unwrap() {
            Stmt { kind: StmtKind::Expr(e), .. } => Body::Expr(Box::new(e)),
            other => Body::Block(vec![other]),
        },
        other => other,
    }
}

fn t_span(t: &o::TSType) -> oxc_span::Span {
    oxc_span::GetSpan::span(t)
}

fn span_of(t: &o::SimpleAssignmentTarget) -> oxc_span::Span {
    oxc_span::GetSpan::span(t)
}

/// The JSX whitespace rule: trim each line, drop empty lines, and join the
/// remaining lines with one space.
fn jsx_text(raw: &str) -> Option<String> {
    let lines: Vec<&str> = raw.split('\n').collect();
    let last = lines.len() - 1;
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        let mut l = line.replace('\t', " ");
        if i != 0 {
            l = l.trim_start().to_string();
        }
        if i != last {
            l = l.trim_end().to_string();
        }
        if l.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&l);
    }
    if out.is_empty() { None } else { Some(out) }
}
