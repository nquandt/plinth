//! JSX: controls of UI API 1.0 and user components (SPEC.md §6.3, §7.2).

use super::{Binding, Checker, ReactiveCtx};
use crate::ast::{Expr, ExprKind, JsxChild, JsxElement};
use crate::controls::{self, ChildKind, PropTy, Target};
use crate::diag::code;
use crate::tir::*;
use crate::types::{FuncType, Type};
use std::rc::Rc;

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
                if let ExprKind::Array(items) = &value.kind {
                    // Literal fast path: join at compile time, and diagnose
                    // a literal item that contains the separator.
                    let mut parts = Vec::new();
                    for (spread, it) in items {
                        match &it.kind {
                            _ if *spread => self.err(code::TYPE_MISMATCH, it.span, "each item must be a string literal"),
                            ExprKind::Str(s) if !s.contains('\u{1f}') => parts.push(s.clone()),
                            ExprKind::Str(_) => self.err(code::TYPE_MISMATCH, it.span, "the character U+001F is not allowed here"),
                            _ => self.err(code::TYPE_MISMATCH, it.span, "each item must be a string literal"),
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
                PropTy::StrList | PropTy::ActionList => unreachable!("handled above with `continue`"),
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
