use rustc_hash::FxHashSet;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum LocalsIdentity {
    #[default]
    ByValue,
    Fresh,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct MarshalHints {
    pub(crate) locals_identity: LocalsIdentity,
    pub(crate) explicitly_interned_strings: FxHashSet<String>,
}

impl MarshalHints {
    /// Compares the subset of marshal hints that participates in code-constant equality.
    pub(crate) fn equal_for_constant_pool(&self, other: &Self) -> bool {
        // CPython gives annotation-scope locals fresh identity, which distinguishes otherwise
        // equal code constants. Explicit intern hints only affect the later marshal traversal and
        // are intentionally omitted from constant-pool equality.
        self.locals_identity == other.locals_identity
    }
}

#[derive(Clone, Debug)]
pub(crate) enum Constant {
    None,
    Bool(bool),
    Ellipsis,
    Int(u64),
    SignedInt(i64),
    BigInt {
        negative: bool,
        digits: Vec<u16>,
    },
    Float(f64),
    Complex {
        real: f64,
        imag: f64,
    },
    String(String),
    SurrogateString(Vec<u8>),
    Bytes(Vec<u8>),
    Tuple(Vec<Self>),
    FrozenSet(Vec<Self>),
    Slice {
        lower: Box<Self>,
        upper: Box<Self>,
        step: Box<Self>,
    },
    Code(Box<CodeObject>),
}

#[derive(Clone, Debug)]
pub(crate) struct CodeObject {
    pub(crate) arg_count: u32,
    pub(crate) positional_only_arg_count: u32,
    pub(crate) keyword_only_arg_count: u32,
    pub(crate) stack_size: u32,
    pub(crate) flags: u32,
    pub(crate) bytecode: Vec<u8>,
    pub(crate) constants: Vec<Constant>,
    pub(crate) names: Vec<String>,
    pub(crate) locals: Vec<String>,
    pub(crate) local_kinds: Vec<u8>,
    pub(crate) filename: String,
    pub(crate) name: String,
    pub(crate) qualified_name: String,
    pub(crate) first_line_number: u32,
    pub(crate) line_table: Vec<u8>,
    pub(crate) exception_table: Vec<u8>,
    pub(crate) marshal_hints: MarshalHints,
}
