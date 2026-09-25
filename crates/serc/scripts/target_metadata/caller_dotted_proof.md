# Initial caller state: scoped dotted-name proof

This proof concerns the exact interpreter, source-only runtime, environment, and
unchanged comparator oracle bound by each admitted companion. Its boundary is
immediately before the oracle compiles the target bytes. The candidate set
overapproximates possible interned names in that initial state. It does not
assert that any candidate is interned or retained.

The domain is
`[A-Za-z_][A-Za-z_0-9]*(?:\.(?:[A-Za-z_][A-Za-z_0-9]*|<locals>))+`. Only the
complement within this domain may supply an initial absence fact. The
independently pinned positive profile remains partial. A generated file's
completion flag cannot authorize its own use.

## Native initialization

The inventory includes the core Objects, Python, Parser and Include translation
units and headers, the initialized built-in/extension module roots, their helper
units, and recursively resolved local includes. The exact runtime config,
Makefile, configuration header and sysconfig data bind build selection.
Extension registration alone does not execute its initializer; optional modules
whose initialization is possible are nevertheless included conservatively.

Full decoded C literals are collected with adjacent-string concatenation. Static
type-module lookup can intern the prefix preceding the last dot in `tp_name`, so
every such literal prefix is included. The finite POSIX `MODNAME` type-name
products are expanded explicitly. Identifier stringification in static Unicode,
module constant, slot, exception and configuration macros receives fixed bare C
identifiers. Diagnostic, assembly and documentation stringification does not
feed a Python interner. Build/version strings from `getbuildinfo.c` become
ordinary `sys.version` and `sys._git` values.

Descriptor and keyword initialization interns declared names. Module-addition
helpers intern their key, or the last component of a type name. Generic full
qualified-name constructors for types, methods and descriptors return ordinary
Unicode values. The relevant imported code/extension filename interners receive
qualified absolute runtime paths or frozen-code labels outside the domain;
target filenames are handled separately.

Generated frozen/getpath arrays are covered by the exact source and freeze
recipes. The qualified runtime excludes bytecode caches, ZIP imports, site
packages, custom hooks and platform-specific startup paths. Generated JIT,
tracing, allocator and bytecode implementation includes are classified against
the exact build, rather than treated as additional Python module initializers.

## Python initialization

Every fixed source/frozen input is compiled by the exact admitted interpreter
without executing it. Recursive code metadata and literal strings are included.
The AST also supplies every normalized dotted import prefix: the parser can
intern intermediate prefixes even when their import statement is optimized out.

Final code constants alone are insufficient because the optimizer can discard
child code after its metadata was interned. A lexical AST inventory therefore
adds raw named function/class paths, nested `<locals>` paths and global-reset
alternatives before pruning. Private-name mangling affects the symbol-table
query for a global reset, not the raw metadata name; both reset possibilities
are retained conservatively. Synthetic annotation, type-alias and type-parameter
helper names are included. Generic/anonymous scope markers outside the declared
grammar do not establish facts within it.

The whole-name call audit covers direct calls and passed aliases of
`sys.intern`, attribute setters, `operator.methodcaller`, codec lookup, code
construction and marshal loading. Namedtuple generation uses fixed validated
type/field recipes. Wrapper metadata copying interns fixed attribute keys, not
the copied values. Enum member and private-slot recipes use parsed simple
class/member names. Importlib parent attachment uses the last module-name
component. `attrgetter` splits dotted paths before interning components. Each
possible target-specific dynamic recipe is recorded with the companion's source
inventory.

The exact initial Python inputs use the qualified UTF-8 decoding paths. Their
identifier and string tokens are reviewed for parser-triggered Unicode imports;
raw and doubled-backslash named-escape spellings do not trigger that import. The
oracle reads the target as bytes without decoding it.

## Composition and limits

Python 3.12 and 3.13 pathname components can be retained by pathlib before the
target compile call. The comparator adds those exact components per input;
retained ownership overrides the scoped absence predicate. Later targets do not
apply the earlier pathlib interning rule. Canonical relative target paths remain
valid and are not confused with imported runtime filenames.

Target-source decoding, non-ASCII identifiers, named Unicode escapes, warnings
and other compilation-triggered effects lie after this proof's boundary. A
subsequent unknown addition invalidates an earlier absence fact unless a
separate effect proof preserves it. Arbitrary interpreter builds, mutable module
bindings, custom callbacks and unsupported import/codec contexts remain outside
the admitted contract. Names outside the declared domain remain unknown.
