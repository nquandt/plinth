//! JSX: controls of UI API 1.0 and user components (SPEC.md §6.3, §7.2).

use super::{Binding, Checker, ReactiveCtx};
use crate::ast::{Expr, ExprKind, JsxChild, JsxElement, ObjProp, UnOp};
use crate::controls::{self, ChildKind, PropSpec, PropTy, Target};
use crate::diag::code;
use crate::tir::*;
use crate::types::{FuncType, Type};
use std::rc::Rc;

fn bx(e: TExpr) -> Box<TExpr> {
    Box::new(e)
}

impl Checker<'_> {
    pub(super) fn jsx(&mut self, el: &JsxElement) -> TJsx {
        // Inside JSX, a signal/computed read is reactive (it re-runs when the
        // value changes), unlike a plain read in the component body.
        let saved = self.fx.reactive;
        if saved != ReactiveCtx::Callback {
            self.fx.reactive = ReactiveCtx::Reactive;
        }
        let result = self.jsx_inner(el);
        self.fx.reactive = saved;
        result
    }

    fn jsx_inner(&mut self, el: &JsxElement) -> TJsx {
        match self.lookup(&el.name) {
            Some(Binding::Control(kind)) => self.control(el, controls::by_kind(kind)),
            Some(Binding::Func(fid)) => self.component(el, fid),
            Some(Binding::Var(v)) if matches!(self.prog.vars[v as usize].ty, Type::Func(_)) => {
                self.err_help(
                    code::UNKNOWN_CONTROL,
                    el.name_span,
                    format!("`{}` is a function value; components must be top-level functions", el.name),
                    "declare it with `function`",
                );
                TJsx::Control { kind: plinth_protocol::ControlKind::Section, props: Vec::new(), children: TChildren::None, span: el.span }
            }
            _ => {
                let known = controls::CONTROLS.iter().any(|c| c.name == el.name);
                let help = if known {
                    format!("import it: `import {{ {} }} from \"plinth:ui\"`", el.name)
                } else {
                    let names: Vec<&str> = controls::CONTROLS.iter().map(|c| c.name).collect();
                    format!("the controls are: {}", names.join(", "))
                };
                self.err_help(code::UNKNOWN_CONTROL, el.name_span, format!("unknown control `<{}>`", el.name), help);
                // Check the children anyway, for more diagnostics.
                for c in &el.children {
                    if let JsxChild::Element(e) = c {
                        self.jsx(e);
                    }
                }
                TJsx::Control { kind: plinth_protocol::ControlKind::Section, props: Vec::new(), children: TChildren::None, span: el.span }
            }
        }
    }

    fn control(&mut self, el: &JsxElement, spec: &'static controls::ControlSpec) -> TJsx {
        let mut props = Vec::new();
        let mut action_children: Vec<TChild> = Vec::new();
        let mut seen: Vec<&str> = Vec::new();
        // `items` first: it gives the item type to `key` and `row`.
        let mut attrs: Vec<_> = el.attrs.iter().collect();
        attrs.sort_by_key(|a| if a.name == "items" { 0 } else { 1 });
        let mut item_ty: Option<Type> = None;
        let mut has_bind = false;
        let mut has_on_change = false;
        for a in attrs {
            let Some(ps) = spec.props.iter().find(|p| p.name == a.name) else {
                let names: Vec<&str> = spec.props.iter().map(|p| p.name).collect();
                let msg = format!("`<{}>` has no prop `{}`", spec.name, a.name);
                let help = if matches!(a.name.as_str(), "style" | "class" | "className" | "color" | "width" | "height" | "padding" | "margin")
                {
                    "the runtime owns layout and styling (SPEC.md §6.1)".to_string()
                } else {
                    format!("the props are: {}", names.join(", "))
                };
                self.err_help(code::UNKNOWN_PROP, a.span, msg, help);
                if let Some(v) = &a.value {
                    self.expr(v, None);
                }
                continue;
            };
            if seen.contains(&ps.name) {
                self.err(code::DUPLICATE, a.span, format!("duplicate prop `{}`", a.name));
                continue;
            }
            seen.push(ps.name);
            let value = match &a.value {
                Some(v) => v.clone(),
                None => Expr { kind: ExprKind::Bool(true), span: a.span },
            };
            if ps.ty == PropTy::StrList {
                let Target::Prop(id) = ps.target else { unreachable!("StrList always targets a prop") };
                // The fast path is for an array literal of string literals
                // only. Any other array (with a `const` name, a spread or a
                // call in it) takes the run-time path below.
                let literal = matches!(&value.kind, ExprKind::Array(items)
                    if items.iter().all(|(spread, it)| !*spread && matches!(it.kind, ExprKind::Str(_))));
                if let (true, ExprKind::Array(items)) = (literal, &value.kind) {
                    // Literal fast path: join at compile time, and diagnose
                    // a literal item that contains the separator.
                    let mut parts = Vec::new();
                    for (_, it) in items {
                        if let ExprKind::Str(s) = &it.kind {
                            if s.contains('\u{1f}') {
                                self.err(code::TYPE_MISMATCH, it.span, "the character U+001F is not allowed here");
                            } else {
                                parts.push(s.clone());
                            }
                        }
                    }
                    let joined = TExpr::new(TExprKind::Str(parts.join("\u{1f}")), Type::String, value.span);
                    props.push(TProp { target: PropTarget::Str(id), value: joined });
                } else {
                    // Any other `string[]` expression (a signal read, a
                    // computed, a plain variable): join at run time with
                    // the same U+001F separator. This is an ordinary
                    // string-valued prop from here on, so it gets the same
                    // reactive-effect treatment as any other prop that
                    // reads a signal in JSX.
                    let te = self.typed(&value, &Type::Array(Box::new(Type::String)));
                    let sep = TExpr::new(TExprKind::Str("\u{1f}".into()), Type::String, value.span);
                    let joined = TExpr::new(TExprKind::Rt("arr_join", vec![te, sep]), Type::String, value.span);
                    props.push(TProp { target: PropTarget::Str(id), value: joined });
                }
                continue;
            }
            if let PropTy::PartialStyle(allowed) = ps.ty {
                let Target::Prop(id) = ps.target else { unreachable!("a partial style always targets a prop") };
                if let Some(text) = self.encode_partial_style(&value, allowed, ps.name) {
                    let te = TExpr::new(TExprKind::Str(text), Type::String, value.span);
                    props.push(TProp { target: PropTarget::Str(id), value: te });
                }
                continue;
            }
            if ps.ty == PropTy::ChartPoints {
                let Target::Prop(id) = ps.target else { unreachable!("ChartPoints always targets a prop") };
                match self.encode_chart_points(&value) {
                    Some(te) => props.push(TProp { target: PropTarget::Str(id), value: te }),
                    None => self.err_help(
                        code::TYPE_MISMATCH,
                        value.span,
                        format!("`{}` must be an array of `{{ label, value }}` objects", ps.name),
                        "write `data={[{ label: \"Jan\", value: total() }]}`, or pass a `ChartPoint[]`",
                    ),
                }
                continue;
            }
            if ps.ty == PropTy::ChartSeries {
                let Target::Prop(id) = ps.target else { unreachable!("ChartSeries always targets a prop") };
                let ExprKind::Array(items) = &value.kind else {
                    match self.encode_chart_series_dyn(&value) {
                        Some(te) => props.push(TProp { target: PropTarget::Str(id), value: te }),
                        None => self.err_help(
                            code::TYPE_MISMATCH,
                            value.span,
                            "`series` must be an array of `{ name, points }` objects",
                            "write `series={[{ name: \"2026\", points: [...] }]}`, or pass a `ChartSeriesDef[]`",
                        ),
                    }
                    continue;
                };
                let mut parts = Vec::new();
                let mut ok = true;
                for (spread, it) in items {
                    if *spread {
                        self.err(code::TYPE_MISMATCH, it.span, "a series cannot be spread");
                        ok = false;
                        continue;
                    }
                    let ExprKind::Object(fields) = &it.kind else {
                        self.err(code::TYPE_MISMATCH, it.span, "each series must be a `{ name, points }` object literal");
                        ok = false;
                        continue;
                    };
                    let mut name_e = None;
                    let mut points_e = None;
                    for f in fields {
                        if let ObjProp::Field(key, e, _) = f {
                            match key.as_str() {
                                "name" => name_e = Some(e),
                                "points" => points_e = Some(e),
                                _ => {}
                            }
                        }
                    }
                    let (Some(name_e), Some(points_e)) = (name_e, points_e) else {
                        self.err(code::TYPE_MISMATCH, it.span, "a series needs `name` and `points`");
                        ok = false;
                        continue;
                    };
                    let name_te = self.typed(name_e, &Type::String);
                    let Some(points_te) = self.encode_chart_points(points_e) else {
                        self.err(code::TYPE_MISMATCH, points_e.span, "`points` must be an array of `{ label, value }` objects");
                        ok = false;
                        continue;
                    };
                    let sep = TExpr::new(TExprKind::Str("\u{1}".into()), Type::String, it.span);
                    parts.push(self.concat3(name_te, sep, points_te, it.span));
                }
                if ok {
                    let joined = self.join_parts(parts, "\u{1e}", value.span);
                    props.push(TProp { target: PropTarget::Str(id), value: joined });
                }
                continue;
            }
            if ps.ty == PropTy::ActionList {
                let ExprKind::Array(items) = &value.kind else {
                    self.err(code::TYPE_MISMATCH, value.span, format!("`{}` must be an array literal of `<Action>` elements", ps.name));
                    continue;
                };
                for (_, it) in items {
                    match &it.kind {
                        ExprKind::Jsx(jel) if jel.name == "Action" => {
                            let j = self.jsx(jel);
                            action_children.push(TChild::Element(j));
                        }
                        _ => self.err_help(
                            code::TYPE_MISMATCH,
                            it.span,
                            "an action must be an `<Action>` element",
                            "write `<Action label=\"...\" onPress={...} />`",
                        ),
                    }
                }
                continue;
            }
            let id = match ps.target {
                Target::Prop(id) | Target::Event(id) => id,
                _ => 0,
            };
            let (target, te) = match ps.ty {
                PropTy::Str => {
                    let te = self.typed(&value, &Type::String);
                    if ps.name == "alt" {
                        match &value.kind {
                            ExprKind::Str(s) if s.trim().is_empty() => self.err_help(
                                code::EMPTY_ALT,
                                value.span,
                                "`alt` must not be empty",
                                "describe the image for people using a screen reader",
                            ),
                            _ => {}
                        }
                    }
                    (PropTarget::Str(id), te)
                }
                PropTy::Asset => {
                    let te = self.typed(&value, &Type::String);
                    match &value.kind {
                        ExprKind::Str(s) if self.assets.contains(s.as_str()) => {}
                        ExprKind::Str(s) => {
                            let mut names: Vec<&str> = self.assets.iter().map(|s| s.as_str()).collect();
                            names.sort_unstable();
                            let help = if names.is_empty() {
                                "the project has no files under assets/".to_string()
                            } else {
                                format!("the assets are: {}", names.join(", "))
                            };
                            self.err_help(code::BAD_ASSET, value.span, format!("there is no asset `{s}`"), help);
                        }
                        _ => self.err(code::TYPE_MISMATCH, value.span, format!("`{}` must be a string literal", ps.name)),
                    }
                    (PropTarget::Str(id), te)
                }
                PropTy::StrOneOf(names) => {
                    let t = Type::str_lits(names.iter().map(|s| s.to_string()).collect());
                    let te = self.typed(&value, &t);
                    (PropTarget::Str(id), self.coerce(te, &Type::String))
                }
                PropTy::Size(fraction_id) => match &value.kind {
                    // A string is a fraction (`"1/2"`, `"full"`, ...): an
                    // enum prop. Anything else is a number of spacing units.
                    ExprKind::Str(_) => {
                        let table = crate::controls::FRACTIONS;
                        let t = Type::str_lits(table.iter().map(|(s, _)| s.to_string()).collect());
                        let te = self.typed(&value, &t);
                        (PropTarget::Enum(fraction_id, table.iter().map(|(s, v)| (s.to_string(), *v)).collect()), te)
                    }
                    _ => (PropTarget::Int(id), self.typed(&value, &Type::Number)),
                },
                PropTy::Num => (PropTarget::Num(id), self.typed(&value, &Type::Number)),
                PropTy::Int => (PropTarget::Int(id), self.typed(&value, &Type::Number)),
                PropTy::Bool => (PropTarget::Bool(id), self.typed(&value, &Type::Bool)),
                PropTy::Enum(table) => {
                    let t = Type::str_lits(table.iter().map(|(s, _)| s.to_string()).collect());
                    let te = self.typed(&value, &t);
                    (PropTarget::Enum(id, table.iter().map(|(s, v)| (s.to_string(), *v)).collect()), te)
                }
                PropTy::Callback0 | PropTy::CallbackStr | PropTy::CallbackBool | PropTy::CallbackNum => {
                    if ps.name == "onChange" {
                        has_on_change = true;
                    }
                    let params = match ps.ty {
                        PropTy::CallbackStr => vec![Type::String],
                        PropTy::CallbackBool => vec![Type::Bool],
                        PropTy::CallbackNum => vec![Type::Number],
                        _ => Vec::new(),
                    };
                    let (f, _) = self.callback(&value, &params, Some(Type::Void));
                    if let Some(ft) = self.func_ty(&f)
                        && ft.params.len() < params.len()
                    {
                        // A shorter callback is fine; the thunk drops the rest.
                    }
                    (PropTarget::Event(id), f)
                }
                PropTy::ValueStr | PropTy::ValueBool | PropTy::ValueNum => {
                    let kind = match ps.ty {
                        PropTy::ValueBool => BindValKind::Bool,
                        PropTy::ValueNum => BindValKind::Num,
                        _ => BindValKind::Str,
                    };
                    let base = match kind {
                        BindValKind::Bool => Type::Bool,
                        BindValKind::Num => Type::Number,
                        BindValKind::Str => Type::String,
                    };
                    let te = self.expr(&value, Some(&base));
                    match &te.ty {
                        Type::Signal(inner) if self.conversion(inner, &base).is_some() && self.conversion(&base, inner).is_some() => {
                            has_bind = true;
                            (PropTarget::Bind { kind }, te)
                        }
                        Type::Signal(inner) => {
                            let msg = format!("`value` needs a `Signal<{}>`, not `Signal<{}>`", self.show(&base), self.show(inner));
                            self.err(code::BAD_BINDING, value.span, msg);
                            (PropTarget::Bind { kind }, te)
                        }
                        Type::Computed(_) => {
                            self.err_help(
                                code::BAD_BINDING,
                                value.span,
                                "a computed value cannot bind both ways",
                                "pass `value={c()}` and handle `onChange`",
                            );
                            (PropTarget::Str(plinth_protocol::prop::VALUE), te)
                        }
                        _ => {
                            let te = self.coerce(te, &base);
                            let target = match kind {
                                BindValKind::Bool => PropTarget::Bool(plinth_protocol::prop::VALUE),
                                BindValKind::Num => PropTarget::Num(plinth_protocol::prop::VALUE),
                                BindValKind::Str => PropTarget::Str(plinth_protocol::prop::VALUE),
                            };
                            (target, te)
                        }
                    }
                }
                PropTy::ListItems => {
                    let te = self.expr(&value, None);
                    match &te.ty {
                        Type::Array(t) => item_ty = Some((**t).clone()),
                        Type::Error => item_ty = Some(Type::Error),
                        other => {
                            let msg = format!("`items` must be an array, not `{}`", self.show(other));
                            let help = if matches!(other, Type::Signal(_) | Type::Computed(_)) {
                                "call it: `items={xs()}`"
                            } else {
                                "pass an array"
                            };
                            self.err_help(code::TYPE_MISMATCH, value.span, msg, help);
                            item_ty = Some(Type::Error);
                        }
                    }
                    (PropTarget::ListItems, te)
                }
                PropTy::ListKey => {
                    let it = item_ty.clone().unwrap_or(Type::Error);
                    let (f, _) = self.callback(&value, &[it], None);
                    if let Some(ft) = self.func_ty(&f)
                        && !matches!(self.widen(ft.ret.clone()), Type::Number | Type::String | Type::Error)
                    {
                        let msg = format!("`key` must return a number or a string, not `{}`", self.show(&ft.ret));
                        self.err(code::TYPE_MISMATCH, value.span, msg);
                    }
                    (PropTarget::ListKey, f)
                }
                PropTy::ListRow => {
                    let it = item_ty.clone().unwrap_or(Type::Error);
                    let (f, _) = self.callback(&value, &[it], Some(Type::Element));
                    (PropTarget::ListRow, f)
                }
                PropTy::Element => (PropTarget::ListEmpty, self.typed(&value, &Type::Element)),
                PropTy::StrList | PropTy::ActionList | PropTy::ChartPoints | PropTy::ChartSeries | PropTy::PartialStyle(_) => {
                    unreachable!("handled above with `continue`")
                }
            };
            props.push(TProp { target, value: te });
        }
        for ps in spec.props {
            if ps.required && !seen.contains(&ps.name) {
                self.err(code::MISSING_PROP, el.name_span, format!("`<{}>` needs the prop `{}`", spec.name, ps.name));
            }
        }
        if has_bind && has_on_change {
            self.err_help(
                code::BAD_BINDING,
                el.name_span,
                "a signal `value` binds both ways, so `onChange` is not allowed with it",
                "remove `onChange`, or pass `value={s()}` and handle `onChange`",
            );
        }
        let mut children = self.children(el, spec);
        if !action_children.is_empty() {
            match &mut children {
                TChildren::Nodes(out) => out.extend(action_children),
                TChildren::None => children = TChildren::Nodes(action_children),
                TChildren::Text(_) => self.err(code::BAD_CHILD, el.name_span, format!("`<{}>` cannot mix text children and `actions`", spec.name)),
            }
        }
        TJsx::Control { kind: spec.kind, props, children, span: el.span }
    }

    fn typed(&mut self, e: &Expr, ty: &Type) -> TExpr {
        let te = self.expr(e, Some(ty));
        self.coerce(te, ty)
    }

    /// `str_concat(a, str_concat(b, c))`, left to right.
    fn concat3(&mut self, a: TExpr, b: TExpr, c: TExpr, span: crate::diag::Span) -> TExpr {
        let bc = TExpr::new(TExprKind::Rt("str_concat", vec![b, c]), Type::String, span);
        TExpr::new(TExprKind::Rt("str_concat", vec![a, bc]), Type::String, span)
    }

    /// Joins already-string `parts` with a literal `sep`, as one `str_concat`
    /// chain. An empty list gives the empty string.
    fn join_parts(&mut self, parts: Vec<TExpr>, sep: &str, span: crate::diag::Span) -> TExpr {
        let mut it = parts.into_iter();
        let Some(mut acc) = it.next() else {
            return TExpr::new(TExprKind::Str(String::new()), Type::String, span);
        };
        for part in it {
            let sep_lit = TExpr::new(TExprKind::Str(sep.to_string()), Type::String, span);
            let with_sep = TExpr::new(TExprKind::Rt("str_concat", vec![acc, sep_lit]), Type::String, span);
            acc = TExpr::new(TExprKind::Rt("str_concat", vec![with_sep, part]), Type::String, span);
        }
        acc
    }

    /// Encodes `Chart.data`/`Chart.series[].points` (SPEC.md §6.3, UI API
    /// 1.5): an array literal of `{ label: string, value: number }` object
    /// literals, into one `"label\u0001value"` string per point, joined
    /// with U+001F. `None` means `value` was not that shape; the caller
    /// reports the diagnostic. The `value` expressions may read signals,
    /// which keeps the resulting prop reactive through the normal JSX
    /// reactive-effect machinery, same as any other string prop.
    /// A Level 2 partial style (UI API 1.7): `{ bg: "hover", padding: 4 }`
    /// becomes `"56:11,42:4"` (prop id and int value). Every value must be a
    /// literal: the style is fixed at compile time.
    fn encode_partial_style(&mut self, value: &Expr, allowed: &[PropSpec], name: &str) -> Option<String> {
        let ExprKind::Object(fields) = &value.kind else {
            self.err_help(
                code::TYPE_MISMATCH,
                value.span,
                format!("`{name}` must be an object literal of style props"),
                format!("write `{name}={{{{ bg: \"hover\" }}}}`"),
            );
            return None;
        };
        let mut pairs: Vec<(u16, i64)> = Vec::new();
        let mut ok = true;
        for f in fields {
            let ObjProp::Field(key, e, key_span) = f else {
                self.err(code::TYPE_MISMATCH, value.span, format!("`{name}` cannot use a spread"));
                ok = false;
                continue;
            };
            let Some(ps) = allowed.iter().find(|p| p.name == key) else {
                let names: Vec<&str> = allowed.iter().map(|p| p.name).collect();
                self.err_help(code::UNKNOWN_PROP, *key_span, format!("`{key}` is not a style prop"), format!("the style props are: {}", names.join(", ")));
                ok = false;
                continue;
            };
            let Target::Prop(id) = ps.target else { continue };
            // A negative number literal is `-` applied to a number.
            let num = match &e.kind {
                ExprKind::Num(n) => Some(*n),
                ExprKind::Unary(UnOp::Neg, inner) => match inner.kind {
                    ExprKind::Num(n) => Some(-n),
                    _ => None,
                },
                _ => None,
            };
            let pair = match (ps.ty, &e.kind, num) {
                (PropTy::Enum(table), ExprKind::Str(s), _) => table.iter().find(|(n, _)| n == s).map(|(_, v)| (id, i64::from(*v))),
                (PropTy::Size(fraction), ExprKind::Str(s), _) => {
                    crate::controls::FRACTIONS.iter().find(|(n, _)| n == s).map(|(_, v)| (fraction, i64::from(*v)))
                }
                (PropTy::Int | PropTy::Size(_), _, Some(n)) if n.fract() == 0.0 && n.abs() < 1e6 => Some((id, n as i64)),
                (PropTy::Bool, ExprKind::Bool(b), _) => Some((id, i64::from(*b))),
                _ => None,
            };
            match pair {
                Some(p) => pairs.push(p),
                None => {
                    let want = match ps.ty {
                        PropTy::Enum(table) => table.iter().map(|(n, _)| format!("\"{n}\"")).collect::<Vec<_>>().join(" | "),
                        PropTy::Size(_) => "a whole number of spacing units, or a fraction such as \"1/2\"".into(),
                        PropTy::Bool => "true or false".into(),
                        _ => "a whole number".into(),
                    };
                    self.err_help(code::TYPE_MISMATCH, e.span, format!("`{key}` in `{name}` must be a literal"), format!("use {want}"));
                    ok = false;
                }
            }
        }
        ok.then(|| pairs.iter().map(|(k, v)| format!("{k}:{v}")).collect::<Vec<_>>().join(","))
    }

    fn encode_chart_points(&mut self, value: &Expr) -> Option<TExpr> {
        let ExprKind::Array(items) = &value.kind else { return self.encode_chart_points_dyn(value) };
        let mut parts = Vec::new();
        for (spread, it) in items {
            if *spread {
                self.err(code::TYPE_MISMATCH, it.span, "a chart point cannot be spread");
                continue;
            }
            let ExprKind::Object(fields) = &it.kind else {
                self.err(code::TYPE_MISMATCH, it.span, "each point must be a `{ label, value }` object literal");
                continue;
            };
            let mut label_e = None;
            let mut value_e = None;
            for f in fields {
                if let ObjProp::Field(key, e, _) = f {
                    match key.as_str() {
                        "label" => label_e = Some(e),
                        "value" => value_e = Some(e),
                        _ => {}
                    }
                }
            }
            let (Some(label_e), Some(value_e)) = (label_e, value_e) else {
                self.err(code::TYPE_MISMATCH, it.span, "a chart point needs `label` and `value`");
                continue;
            };
            let label_te = self.typed(label_e, &Type::String);
            let num_te = self.typed(value_e, &Type::Number);
            let num_str = TExpr::new(TExprKind::Rt("json_num_str", vec![num_te]), Type::String, value_e.span);
            let sep = TExpr::new(TExprKind::Str("\u{1}".into()), Type::String, it.span);
            parts.push(self.concat3(label_te, sep, num_str, it.span));
        }
        Some(self.join_parts(parts, "\u{1f}", value.span))
    }

    /// `Chart.data`/`points` as any other expression (a variable, a signal
    /// read, a `.map()` result): an array of structs with a `label: string`
    /// and a `value: number` field (`ChartPoint`, or any object type with
    /// these fields). The encoding is the same as for a literal, built at
    /// run time with a loop. It is an ordinary string prop, so a signal
    /// read in `value` keeps the prop reactive. `None` means another type;
    /// the caller reports the diagnostic.
    fn encode_chart_points_dyn(&mut self, value: &Expr) -> Option<TExpr> {
        let arr = self.expr(value, None);
        self.encode_points_texpr(arr)
    }

    /// The point encoding for a checked array of `{ label, value }` structs.
    fn encode_points_texpr(&mut self, arr: TExpr) -> Option<TExpr> {
        let span = arr.span;
        let Type::Array(elem) = &arr.ty else { return None };
        let Type::Struct(sid) = **elem else { return None };
        let def = &self.prog.structs[sid as usize];
        let (label_i, label_ty) = def.field("label").map(|(i, f)| (i as u32, f.ty.clone()))?;
        let (value_i, value_ty) = def.field("value").map(|(i, f)| (i as u32, f.ty.clone()))?;
        if !label_ty.is_stringish() || !matches!(value_ty, Type::Number | Type::Int) {
            return None;
        }
        self.encode_each(arr, "\u{1f}", |c, pt| {
            let label = TExpr::new(TExprKind::Field(bx(pt.clone()), sid, label_i), label_ty.clone(), span);
            let label = c.coerce(label, &Type::String);
            let v = TExpr::new(TExprKind::Field(bx(pt), sid, value_i), value_ty.clone(), span);
            let v = c.coerce(v, &Type::Number);
            let v_str = TExpr::new(TExprKind::Rt("json_num_str", vec![v]), Type::String, span);
            let sep = TExpr::new(TExprKind::Str("\u{1}".into()), Type::String, span);
            Some(c.concat3(label, sep, v_str, span))
        })
    }

    /// `Chart.series` as any other expression (a variable, a `computed`, a
    /// `.map()` result): an array of structs with a `name: string` and a
    /// `points` array of `{ label, value }` structs (`ChartSeriesDef`). The
    /// encoding is the same as for a literal, built at run time.
    fn encode_chart_series_dyn(&mut self, value: &Expr) -> Option<TExpr> {
        let arr = self.expr(value, None);
        let span = arr.span;
        let Type::Array(elem) = &arr.ty else { return None };
        let Type::Struct(sid) = **elem else { return None };
        let def = &self.prog.structs[sid as usize];
        let (name_i, name_ty) = def.field("name").map(|(i, f)| (i as u32, f.ty.clone()))?;
        let (points_i, points_ty) = def.field("points").map(|(i, f)| (i as u32, f.ty.clone()))?;
        if !name_ty.is_stringish() {
            return None;
        }
        self.encode_each(arr, "\u{1e}", |c, s| {
            let name = TExpr::new(TExprKind::Field(bx(s.clone()), sid, name_i), name_ty.clone(), span);
            let name = c.coerce(name, &Type::String);
            let points = TExpr::new(TExprKind::Field(bx(s), sid, points_i), points_ty.clone(), span);
            let points = c.encode_points_texpr(points)?;
            let sep = TExpr::new(TExprKind::Str("\u{1}".into()), Type::String, span);
            Some(c.concat3(name, sep, points, span))
        })
    }

    /// A loop over the array `arr` that joins `piece(element)` with `sep`.
    /// `None` when `piece` gives `None` (a wrong element type).
    fn encode_each(&mut self, arr: TExpr, sep: &str, piece: impl FnOnce(&mut Self, TExpr) -> Option<TExpr>) -> Option<TExpr> {
        let span = arr.span;
        let arr_ty = arr.ty.clone();
        let Type::Array(elem) = &arr_ty else { return None };
        let var = |v: VarId, ty: &Type| TExpr::new(TExprKind::Var(v), ty.clone(), span);
        let str_lit = |t: &str| TExpr::new(TExprKind::Str(t.into()), Type::String, span);
        let num = |n: f64| TExpr::new(TExprKind::Num(n), Type::Number, span);
        let arr_v = self.temp(arr_ty.clone());
        let len_v = self.temp(Type::Number);
        let i_v = self.temp(Type::Number);
        let pt_v = self.temp((**elem).clone());
        let acc_v = self.temp(Type::String);
        let i_r = var(i_v, &Type::Number);
        let len = TExpr::new(
            TExprKind::Coerce(Coercion::I32ToNum, bx(TExpr::new(TExprKind::Rt("arr_len", vec![var(arr_v, &arr_ty)]), Type::Bool, span))),
            Type::Number,
            span,
        );
        let get = TExpr::new(TExprKind::Index(bx(var(arr_v, &arr_ty)), bx(i_r.clone())), (**elem).clone(), span);
        let piece = piece(self, var(pt_v, elem))?;
        let append = |acc_v: VarId, e: TExpr| {
            let sum = TExpr::new(TExprKind::Rt("str_concat", vec![TExpr::new(TExprKind::Var(acc_v), Type::String, span), e]), Type::String, span);
            TStmt::Expr(TExpr::new(TExprKind::Assign(Place::Var(acc_v), bx(sum)), Type::String, span))
        };
        let gt0 = TExpr::new(TExprKind::Cmp(CmpOp::Gt, EqKind::F64, bx(i_r.clone()), bx(num(0.0))), Type::Bool, span);
        let body = vec![
            TStmt::Let(pt_v, Some(get)),
            TStmt::If(gt0, vec![append(acc_v, str_lit(sep))], Vec::new()),
            append(acc_v, piece),
        ];
        let cond = TExpr::new(TExprKind::Cmp(CmpOp::Lt, EqKind::F64, bx(i_r.clone()), bx(var(len_v, &Type::Number))), Type::Bool, span);
        let inc = TExpr::new(TExprKind::Num2(NumOp::Add, bx(i_r), bx(num(1.0))), Type::Number, span);
        let update = TExpr::new(TExprKind::Assign(Place::Var(i_v), bx(inc)), Type::Number, span);
        let id = self.prog.new_loop();
        let stmts = vec![
            TStmt::Let(arr_v, Some(arr)),
            TStmt::Let(len_v, Some(len)),
            TStmt::Let(acc_v, Some(str_lit(""))),
            TStmt::Let(i_v, Some(num(0.0))),
            TStmt::Loop { id, cond: Some(cond), test_after: false, update: Some(update), body },
        ];
        Some(TExpr::new(TExprKind::Block(stmts, bx(var(acc_v, &Type::String))), Type::String, span))
    }

    fn children(&mut self, el: &JsxElement, spec: &controls::ControlSpec) -> TChildren {
        match spec.children {
            ChildKind::None => {
                if let Some(c) = el.children.first() {
                    let span = match c {
                        JsxChild::Text(_, s) => *s,
                        JsxChild::Expr(e) => e.span,
                        JsxChild::Element(e) => e.span,
                    };
                    let help = if spec.name == "Button" { "use the `label` prop" } else { "remove the children" };
                    self.err_help(code::BAD_CHILD, span, format!("`<{}>` takes no children", spec.name), help);
                }
                TChildren::None
            }
            ChildKind::Text => {
                let mut parts = Vec::new();
                for c in &el.children {
                    match c {
                        JsxChild::Text(s, span) => parts.push(TExpr::new(TExprKind::Str(s.clone()), Type::String, *span)),
                        JsxChild::Expr(e) => {
                            let te = self.expr(e, None);
                            parts.push(self.to_str(te));
                        }
                        JsxChild::Element(e) => {
                            self.err_help(
                                code::BAD_CHILD,
                                e.span,
                                format!("`<{}>` takes text, not elements", spec.name),
                                "put the elements next to it in a <Section>",
                            );
                        }
                    }
                }
                TChildren::Text(parts)
            }
            ChildKind::Nodes => {
                let mut out = Vec::new();
                for c in &el.children {
                    match c {
                        JsxChild::Element(e) => out.push(TChild::Element(self.jsx(e))),
                        JsxChild::Expr(e) => {
                            // `{items.map(x => <Row .../>)}` (dogfooding
                            // gap #2) desugars to the same keyed `List`
                            // reconciler a literal `<List>` lowers to,
                            // keyed by position, before the generic
                            // "array is not a child" diagnostic below
                            // would otherwise fire for it.
                            if let Some(list) = self.map_child(e) {
                                out.push(TChild::Element(list));
                                continue;
                            }
                            let te = self.expr(e, Some(&Type::Element));
                            if let Type::Array(_) = te.ty {
                                self.err_help(
                                    code::BAD_CHILD,
                                    e.span,
                                    "an array of elements is not a child",
                                    "use <List items={...} key={...} row={...} />",
                                );
                                continue;
                            }
                            let te = self.coerce(te, &Type::Element);
                            out.push(TChild::Expr(te));
                        }
                        JsxChild::Text(_, span) => {
                            self.err_help(
                                code::BAD_CHILD,
                                *span,
                                format!("`<{}>` cannot hold text directly", spec.name),
                                "wrap the text in <Text>",
                            );
                        }
                    }
                }
                TChildren::Nodes(out)
            }
        }
    }

    /// Recognizes `arr.map(x => <Row .../>)` (optionally preceded by a
    /// `.filter(...)`, since `filter` already returns a plain array) as a
    /// JSX child and desugars it to a `<List>` control, keyed by position
    /// — the same keyed reconciler a literal `<List items key row>`
    /// lowers to (`lower.rs`'s `PropTarget::List*` handling, the runtime
    /// `list`/`run_list`). Only this exact shape is recognized: a `.map`
    /// call whose argument is an inline arrow/function expression. A
    /// named function reference, or any other array-producing
    /// expression, falls through to the ordinary "an array of elements
    /// is not a child" diagnostic.
    fn map_child(&mut self, e: &Expr) -> Option<TJsx> {
        let ExprKind::Call { callee, args, optional: false, .. } = &e.kind else { return None };
        let ExprKind::Member { obj, prop, optional: false, .. } = &callee.kind else { return None };
        if prop != "map" || args.len() != 1 {
            return None;
        }
        if !matches!(&args[0].kind, ExprKind::Func(_)) {
            return None;
        }
        let ExprKind::Func(row_fn) = &args[0].kind else { unreachable!() };
        let items_te = self.expr(obj, None);
        let item_ty = match &items_te.ty {
            Type::Array(t) => (**t).clone(),
            Type::Error => Type::Error,
            other => {
                let msg = format!("`.map` here needs an array, not `{}`", self.show(other));
                self.err_help(code::TYPE_MISMATCH, obj.span, msg, "use <List items={...} key={...} row={...} /> for anything more complex");
                return Some(TJsx::Control {
                    kind: plinth_protocol::ControlKind::Section,
                    props: Vec::new(),
                    children: TChildren::None,
                    span: e.span,
                });
            }
        };
        let row_expr = Expr { kind: ExprKind::Func(row_fn.clone()), span: args[0].span };
        let (row, _) = self.callback(&row_expr, &[item_ty.clone()], Some(Type::Element));
        let key_fn = index_key_fn(e.span);
        let key_expr = Expr { kind: ExprKind::Func(Box::new(key_fn)), span: e.span };
        let (key, _) = self.callback(&key_expr, &[item_ty, Type::Number], None);
        let props = vec![
            TProp { target: PropTarget::ListItems, value: items_te },
            TProp { target: PropTarget::ListKey, value: key },
            TProp { target: PropTarget::ListRow, value: row },
        ];
        Some(TJsx::Control { kind: plinth_protocol::ControlKind::List, props, children: TChildren::None, span: e.span })
    }

    fn component(&mut self, el: &JsxElement, fid: FuncId) -> TJsx {
        let ft: Rc<FuncType> = self.func_type(fid, el.name_span);
        if ft.ret != Type::Element && !ft.ret.is_error() {
            let msg = format!("`{}` does not return an element", el.name);
            self.err(code::UNKNOWN_CONTROL, el.name_span, msg);
        }
        if !el.children.is_empty() {
            self.err_help(
                code::BAD_CHILD,
                el.span,
                "children of user components are not supported yet",
                format!("pass the content as a prop to `{}` instead", el.name),
            );
        }
        let props = match ft.params.as_slice() {
            [] => {
                if let Some(a) = el.attrs.first() {
                    self.err(code::UNKNOWN_PROP, a.span, format!("`{}` takes no props", el.name));
                }
                None
            }
            [Type::Struct(sid)] => {
                let sid = *sid;
                let props: Vec<crate::ast::ObjProp> = el
                    .attrs
                    .iter()
                    .map(|a| {
                        let v = a.value.clone().unwrap_or(Expr { kind: ExprKind::Bool(true), span: a.span });
                        crate::ast::ObjProp::Field(a.name.clone(), v, a.span)
                    })
                    .collect();
                let obj = Expr { kind: ExprKind::Object(props), span: el.span };
                let te = self.expr(&obj, Some(&Type::Struct(sid)));
                Some(self.coerce(te, &Type::Struct(sid)))
            }
            [Type::Error] => None,
            _ => {
                self.err_help(
                    code::UNKNOWN_CONTROL,
                    el.name_span,
                    format!("`{}` must take one props object", el.name),
                    "write `function X(props: { a: string })`",
                );
                None
            }
        };
        TJsx::Component { func: fid, props, span: el.span }
    }
}

/// `(item, index) => index`: the synthetic key function for `map_child`'s
/// desugared `<List>`, keyed by position. Built as an AST node (rather
/// than directly as `TExpr`) so it goes through the normal closure
/// type-checking path (`Checker::closure`), which infers each
/// unannotated parameter's type from the `FuncType` offered by the
/// caller's `callback(..., &[item_ty, Type::Number], ...)` call.
fn index_key_fn(span: crate::diag::Span) -> crate::ast::FuncDecl {
    use crate::ast::{Body, FuncDecl, Param, Pattern};
    FuncDecl {
        name: None,
        params: vec![
            Param { pattern: Pattern::Ident("__item".to_string(), span), ty: None, default: None, optional: false, span },
            Param { pattern: Pattern::Ident("__index".to_string(), span), ty: None, default: None, optional: false, span },
        ],
        ret: None,
        body: Body::Expr(Box::new(Expr { kind: ExprKind::Ident("__index".to_string()), span })),
        exported: false,
        is_default: false,
        span,
        type_params: Vec::new(),
        is_async: false,
    }
}
