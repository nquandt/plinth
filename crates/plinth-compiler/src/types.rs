//! Plinth TS types (SPEC.md §4.2) and their Wasm representation.

use std::fmt;
use std::rc::Rc;

pub type StructId = u32;
pub type EnumId = u32;

#[derive(Clone, Debug, PartialEq)]
pub enum Type {
    Number,
    Bool,
    String,
    /// A union of string literals. At run time it is a string.
    StrLits(Rc<[String]>),
    Void,
    Null,
    /// `T | null`. Numbers are boxed; references use 0 for null.
    Nullable(Box<Type>),
    Array(Box<Type>),
    Struct(StructId),
    Func(Rc<FuncType>),
    Signal(Box<Type>),
    Computed(Box<Type>),
    /// A node of the semantic tree. 0 means "no element" (`null`, `false`).
    Element,
    Enum(EnumId),
    App,
    /// The type of an expression with an error. It is compatible with all
    /// types, so one error does not cause more.
    Error,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FuncType {
    pub params: Vec<Type>,
    /// The number of required parameters. The others are optional.
    pub required: usize,
    pub ret: Type,
}

/// The Wasm representation of a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Repr {
    F64,
    /// A plain i32: booleans, enums, node ids, signal handles.
    I32,
    /// A heap reference (an i32 address that the GC traces).
    Ref,
    Void,
}

impl Repr {
    pub fn val_type(self) -> Option<wasm_encoder::ValType> {
        match self {
            Repr::F64 => Some(wasm_encoder::ValType::F64),
            Repr::I32 | Repr::Ref => Some(wasm_encoder::ValType::I32),
            Repr::Void => None,
        }
    }
}

impl Type {
    pub fn repr(&self) -> Repr {
        match self {
            Type::Number => Repr::F64,
            Type::Bool | Type::Element | Type::Enum(_) | Type::Signal(_) | Type::Computed(_) | Type::App => Repr::I32,
            Type::String
            | Type::StrLits(_)
            | Type::Null
            | Type::Nullable(_)
            | Type::Array(_)
            | Type::Struct(_)
            | Type::Func(_) => Repr::Ref,
            Type::Void => Repr::Void,
            Type::Error => Repr::I32,
        }
    }

    pub fn is_stringish(&self) -> bool {
        matches!(self, Type::String | Type::StrLits(_))
    }

    pub fn is_error(&self) -> bool {
        matches!(self, Type::Error)
    }

    pub fn nullable(self) -> Type {
        match self {
            Type::Nullable(_) | Type::Null | Type::Error | Type::Element => self,
            t => Type::Nullable(Box::new(t)),
        }
    }

    pub fn str_lits(mut lits: Vec<String>) -> Type {
        lits.sort();
        lits.dedup();
        Type::StrLits(lits.into())
    }
}

/// A struct layout: fields in declaration order.
#[derive(Clone, Debug)]
pub struct StructDef {
    pub name: String,
    pub fields: Vec<Field>,
}

#[derive(Clone, Debug)]
pub struct Field {
    pub name: String,
    pub ty: Type,
    pub optional: bool,
}

impl StructDef {
    pub fn field(&self, name: &str) -> Option<(usize, &Field)> {
        self.fields.iter().enumerate().find(|(_, f)| f.name == name)
    }
}

#[derive(Clone, Debug)]
pub struct EnumDef {
    pub name: String,
    pub members: Vec<(String, i32)>,
}

/// Prints types in TS syntax for diagnostics.
pub struct Display<'a> {
    pub ty: &'a Type,
    pub structs: &'a [StructDef],
    pub enums: &'a [EnumDef],
}

impl fmt::Display for Display<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sub = |ty| Display { ty, structs: self.structs, enums: self.enums };
        match self.ty {
            Type::Number => write!(f, "number"),
            Type::Bool => write!(f, "boolean"),
            Type::String => write!(f, "string"),
            Type::StrLits(l) => {
                let parts: Vec<String> = l.iter().map(|s| format!("{s:?}")).collect();
                write!(f, "{}", parts.join(" | "))
            }
            Type::Void => write!(f, "void"),
            Type::Null => write!(f, "null"),
            Type::Nullable(t) => write!(f, "{} | null", sub(t)),
            Type::Array(t) => match **t {
                Type::Nullable(_) | Type::StrLits(_) | Type::Func(_) => write!(f, "({})[]", sub(t)),
                _ => write!(f, "{}[]", sub(t)),
            },
            Type::Struct(id) => write!(f, "{}", self.structs[*id as usize].name),
            Type::Func(ft) => {
                write!(f, "(")?;
                for (i, p) in ft.params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    let opt = if i >= ft.required { "?" } else { "" };
                    write!(f, "p{i}{opt}: {}", sub(p))?;
                }
                write!(f, ") => {}", sub(&ft.ret))
            }
            Type::Signal(t) => write!(f, "Signal<{}>", sub(t)),
            Type::Computed(t) => write!(f, "Computed<{}>", sub(t)),
            Type::Element => write!(f, "JSX.Element"),
            Type::Enum(id) => write!(f, "{}", self.enums[*id as usize].name),
            Type::App => write!(f, "App"),
            Type::Error => write!(f, "<error>"),
        }
    }
}
