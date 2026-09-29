# Changelog

## 0.1.1

Released on 2026-09-29.

### Other changes

- Avoid a redundant NOP before module annotations ([#894](https://github.com/astral-sh/serc/pull/894))
- Compile nested continues in finally suites ([#825](https://github.com/astral-sh/serc/pull/825))
- Correct await diagnostics in synchronous functions ([#896](https://github.com/astral-sh/serc/pull/896))
- Enforce the default decimal integer literal limit ([#835](https://github.com/astral-sh/serc/pull/835))
- Exclude normal finalizer markers after branching try bodies ([#909](https://github.com/astral-sh/serc/pull/909))
- Fix comprehension yield ordering before async generator returns ([#895](https://github.com/astral-sh/serc/pull/895))
- Fix optimized bytecode generation and add a minimal CPython reference ([#912](https://github.com/astral-sh/serc/pull/912))
- Guard recursive expression traversal and AST cleanup ([#827](https://github.com/astral-sh/serc/pull/827))
- Improve compilation performance ([#848](https://github.com/astral-sh/serc/pull/848))
- Include duplicate type parameter names in diagnostics ([#850](https://github.com/astral-sh/serc/pull/850))
- Link historical compatibility measurements from Git ([#918](https://github.com/astral-sh/serc/pull/918))
- Match attribute assignment expression diagnostics ([#871](https://github.com/astral-sh/serc/pull/871))
- Match duplicate mapping key diagnostics ([#857](https://github.com/astral-sh/serc/pull/857))
- Match misplaced future import diagnostics ([#860](https://github.com/astral-sh/serc/pull/860))
- Match unknown future feature diagnostics ([#858](https://github.com/astral-sh/serc/pull/858))
- Match yield from tuple syntax diagnostics ([#872](https://github.com/astral-sh/serc/pull/872))
- Model integer limits in duplicate mapping diagnostics ([#887](https://github.com/astral-sh/serc/pull/887))
- Move static compatibility tests into discovered fixtures ([#914](https://github.com/astral-sh/serc/pull/914))
- Preserve annotation setup after negated conditions ([#906](https://github.com/astral-sh/serc/pull/906))
- Preserve assertion success locations in finalizers ([#907](https://github.com/astral-sh/serc/pull/907))
- Preserve async assertion exits on Python 3.12 ([#889](https://github.com/astral-sh/serc/pull/889))
- Preserve enclosing cleanup for nested no-op finalizers ([#900](https://github.com/astral-sh/serc/pull/900))
- Preserve generator exception ownership for nested finalizers ([#903](https://github.com/astral-sh/serc/pull/903))
- Preserve handlers around nonterminal no-op finalizers ([#908](https://github.com/astral-sh/serc/pull/908))
- Preserve native startup intern identity in Python 3.14 ([#830](https://github.com/astral-sh/serc/pull/830))
- Preserve nested finally marker exception coverage ([#904](https://github.com/astral-sh/serc/pull/904))
- Preserve nested handler cleanup on exceptional finally breaks ([#829](https://github.com/astral-sh/serc/pull/829))
- Preserve surrogate mapping-pattern keys ([#877](https://github.com/astral-sh/serc/pull/877))
- Preserve type alias diagnostic precedence ([#867](https://github.com/astral-sh/serc/pull/867))
- Prioritize comprehension yield errors over bare stars ([#885](https://github.com/astral-sh/serc/pull/885))
- Prioritize misplaced future import diagnostics ([#865](https://github.com/astral-sh/serc/pull/865))
- Reject commas after bare yield expressions ([#846](https://github.com/astral-sh/serc/pull/846))
- Reject equal large numeric mapping keys ([#861](https://github.com/astral-sh/serc/pull/861))
- Reject excessive indentation in Python sources ([#834](https://github.com/astral-sh/serc/pull/834))
- Reject parameter specification defaults on Python 3.12 ([#869](https://github.com/astral-sh/serc/pull/869))
- Reject repeated explicit call keywords ([#845](https://github.com/astral-sh/serc/pull/845))
- Reject starred type parameter defaults on Python 3.12 ([#870](https://github.com/astral-sh/serc/pull/870))
- Report direct dict-comprehension value yield errors first ([#880](https://github.com/astral-sh/serc/pull/880))
- Report nested comprehension yield errors in operand order ([#884](https://github.com/astral-sh/serc/pull/884))
- Report yield diagnostics in comprehensions ([#879](https://github.com/astral-sh/serc/pull/879))
- Require explicit caller state for strict corpus comparisons ([#910](https://github.com/astral-sh/serc/pull/910))
- Resolve declarations before pattern errors ([#881](https://github.com/astral-sh/serc/pull/881))
- Resolve private type parameter aliases in generic class headers ([#828](https://github.com/astral-sh/serc/pull/828))
- Reuse assembler instruction and block traversal helpers ([#916](https://github.com/astral-sh/serc/pull/916))
- Reuse corpus hashing in the benchmark runner ([#919](https://github.com/astral-sh/serc/pull/919))
- Select native build profiles for floating-point folding ([#824](https://github.com/astral-sh/serc/pull/824))
- Share CLI compatibility test helpers ([#915](https://github.com/astral-sh/serc/pull/915))
- Share comprehension AST extraction ([#922](https://github.com/astral-sh/serc/pull/922))
- Share library compatibility oracle helpers ([#920](https://github.com/astral-sh/serc/pull/920))
- Share optimization runtime assertions ([#921](https://github.com/astral-sh/serc/pull/921))
- Support Python optimization levels ([#841](https://github.com/astral-sh/serc/pull/841))
- Support explicit decimal literal parsing limits ([#911](https://github.com/astral-sh/serc/pull/911))
- Support hash-based bytecode caches ([#840](https://github.com/astral-sh/serc/pull/840))
- Use the minimal CPython reference for corpus comparisons ([#917](https://github.com/astral-sh/serc/pull/917))
- Validate class-visible annotation scopes on Python 3.12 ([#831](https://github.com/astral-sh/serc/pull/831))
- Validate reserved debug attributes and keywords ([#847](https://github.com/astral-sh/serc/pull/847))

## 0.1.0

Released on 2026-09-25.

Serc’s first release provides an experimental Rust compiler for producing
CPython-compatible bytecode and `.pyc` files without a CPython installation.

### Enhancements

- Target Python 3.12, 3.13, 3.14, or 3.15 from one compiler (astral-sh/serc#447;
https://github.com/astral-sh/serc/pull/447) (astral-sh/serc#503;
https://github.com/astral-sh/serc/pull/503) (astral-sh/serc#563;
https://github.com/astral-sh/serc/pull/563)
- Embed compilation and marshaling through reusable Rust source-byte APIs
(astral-sh/serc#21; https://github.com/astral-sh/serc/pull/21)
- Use the compiler as a library without depending on the separate CLI package
(astral-sh/serc#24; https://github.com/astral-sh/serc/pull/24)
- Compile source encoded with legacy Python codecs or normalized UTF-8 signature
aliases (astral-sh/serc#100; https://github.com/astral-sh/serc/pull/100)
(astral-sh/serc#528; https://github.com/astral-sh/serc/pull/528)
- Compile Unix paths containing surrogate-escaped filenames (astral-sh/serc#667;
https://github.com/astral-sh/serc/pull/667)
- Preserve structured parse errors for library callers (astral-sh/serc#66;
https://github.com/astral-sh/serc/pull/66)
- Generate CPython-compatible cache filenames for explicitly named source files
(astral-sh/serc#436; https://github.com/astral-sh/serc/pull/436)
- Prefer the Serc executable installed alongside the imported Python package
(astral-sh/serc#714; https://github.com/astral-sh/serc/pull/714)

### Bug fixes

- Produce byte-for-byte CPython-compatible output across an extensive public
corpus, including NumPy (astral-sh/serc#418;
https://github.com/astral-sh/serc/pull/418) (astral-sh/serc#534;
https://github.com/astral-sh/serc/pull/534) (astral-sh/serc#546;
https://github.com/astral-sh/serc/pull/546) (astral-sh/serc#547;
https://github.com/astral-sh/serc/pull/547) (astral-sh/serc#548;
https://github.com/astral-sh/serc/pull/548) (astral-sh/serc#549;
https://github.com/astral-sh/serc/pull/549) (astral-sh/serc#550;
https://github.com/astral-sh/serc/pull/550) (astral-sh/serc#551;
https://github.com/astral-sh/serc/pull/551) (astral-sh/serc#552;
https://github.com/astral-sh/serc/pull/552)
- Match CPython bytecode, exception tables, and marshal identity for NumPy
initialization, I/O, testing, and Fortran parsing modules (astral-sh/serc#149;
https://github.com/astral-sh/serc/pull/149) (astral-sh/serc#160;
https://github.com/astral-sh/serc/pull/160) (astral-sh/serc#168;
https://github.com/astral-sh/serc/pull/168) (astral-sh/serc#169;
https://github.com/astral-sh/serc/pull/169) (astral-sh/serc#170;
https://github.com/astral-sh/serc/pull/170) (astral-sh/serc#171;
https://github.com/astral-sh/serc/pull/171)
- Reject invalid context-dependent syntax, declarations, match patterns,
generator returns, and `except*` control-flow exits before lowering
(astral-sh/serc#338; https://github.com/astral-sh/serc/pull/338)
(astral-sh/serc#443; https://github.com/astral-sh/serc/pull/443)
(astral-sh/serc#584; https://github.com/astral-sh/serc/pull/584)
(astral-sh/serc#601; https://github.com/astral-sh/serc/pull/601)
(astral-sh/serc#604; https://github.com/astral-sh/serc/pull/604)
(astral-sh/serc#662; https://github.com/astral-sh/serc/pull/662)
(astral-sh/serc#668; https://github.com/astral-sh/serc/pull/668)
(astral-sh/serc#669; https://github.com/astral-sh/serc/pull/669)
- Match CPython errors for null bytes, embedded-null filenames, decoded leading
byte-order marks, surrogate-containing docstrings, and excessive delimiter
nesting (astral-sh/serc#566; https://github.com/astral-sh/serc/pull/566)
(astral-sh/serc#653; https://github.com/astral-sh/serc/pull/653)
(astral-sh/serc#661; https://github.com/astral-sh/serc/pull/661)
(astral-sh/serc#686; https://github.com/astral-sh/serc/pull/686)
(astral-sh/serc#784; https://github.com/astral-sh/serc/pull/784)
- Preserve pending returns and exception state while unwinding `finally` suites
(astral-sh/serc#571; https://github.com/astral-sh/serc/pull/571)
(astral-sh/serc#687; https://github.com/astral-sh/serc/pull/687)
(astral-sh/serc#729; https://github.com/astral-sh/serc/pull/729)
(astral-sh/serc#737; https://github.com/astral-sh/serc/pull/737)
(astral-sh/serc#818; https://github.com/astral-sh/serc/pull/818)
(astral-sh/serc#823; https://github.com/astral-sh/serc/pull/823)
- Preserve context-manager cleanup and fallthrough across nested handlers,
coroutines, and async generators (astral-sh/serc#754;
https://github.com/astral-sh/serc/pull/754) (astral-sh/serc#758;
https://github.com/astral-sh/serc/pull/758) (astral-sh/serc#759;
https://github.com/astral-sh/serc/pull/759) (astral-sh/serc#761;
https://github.com/astral-sh/serc/pull/761) (astral-sh/serc#762;
https://github.com/astral-sh/serc/pull/762) (astral-sh/serc#763;
https://github.com/astral-sh/serc/pull/763)
- Preserve load ownership while recovering through nested exception and loop
exits (astral-sh/serc#766; https://github.com/astral-sh/serc/pull/766)
(astral-sh/serc#768; https://github.com/astral-sh/serc/pull/768)
(astral-sh/serc#770; https://github.com/astral-sh/serc/pull/770)
(astral-sh/serc#772; https://github.com/astral-sh/serc/pull/772)
(astral-sh/serc#774; https://github.com/astral-sh/serc/pull/774)
(astral-sh/serc#775; https://github.com/astral-sh/serc/pull/775)
(astral-sh/serc#777; https://github.com/astral-sh/serc/pull/777)
- Resolve closures, private names, globals, and local-slot ordering correctly in
nested comprehensions and generators (astral-sh/serc#650;
https://github.com/astral-sh/serc/pull/650) (astral-sh/serc#665;
https://github.com/astral-sh/serc/pull/665) (astral-sh/serc#693;
https://github.com/astral-sh/serc/pull/693) (astral-sh/serc#786;
https://github.com/astral-sh/serc/pull/786) (astral-sh/serc#788;
https://github.com/astral-sh/serc/pull/788) (astral-sh/serc#793;
https://github.com/astral-sh/serc/pull/793) (astral-sh/serc#794;
https://github.com/astral-sh/serc/pull/794) (astral-sh/serc#810;
https://github.com/astral-sh/serc/pull/810)
- Preserve class, generic, and deferred-annotation scopes, captures, future
flags, and qualified names (astral-sh/serc#467;
https://github.com/astral-sh/serc/pull/467) (astral-sh/serc#469;
https://github.com/astral-sh/serc/pull/469) (astral-sh/serc#477;
https://github.com/astral-sh/serc/pull/477) (astral-sh/serc#486;
https://github.com/astral-sh/serc/pull/486) (astral-sh/serc#524;
https://github.com/astral-sh/serc/pull/524) (astral-sh/serc#525;
https://github.com/astral-sh/serc/pull/525) (astral-sh/serc#526;
https://github.com/astral-sh/serc/pull/526) (astral-sh/serc#567;
https://github.com/astral-sh/serc/pull/567) (astral-sh/serc#607;
https://github.com/astral-sh/serc/pull/607) (astral-sh/serc#608;
https://github.com/astral-sh/serc/pull/608) (astral-sh/serc#651;
https://github.com/astral-sh/serc/pull/651) (astral-sh/serc#694;
https://github.com/astral-sh/serc/pull/694) (astral-sh/serc#695;
https://github.com/astral-sh/serc/pull/695) (astral-sh/serc#696;
https://github.com/astral-sh/serc/pull/696) (astral-sh/serc#735;
https://github.com/astral-sh/serc/pull/735)
- Match CPython constant folding for large integers, mixed numeric operations,
powers, division, and non-finite complex values (astral-sh/serc#476;
https://github.com/astral-sh/serc/pull/476) (astral-sh/serc#479;
https://github.com/astral-sh/serc/pull/479) (astral-sh/serc#480;
https://github.com/astral-sh/serc/pull/480) (astral-sh/serc#481;
https://github.com/astral-sh/serc/pull/481) (astral-sh/serc#487;
https://github.com/astral-sh/serc/pull/487) (astral-sh/serc#490;
https://github.com/astral-sh/serc/pull/490) (astral-sh/serc#568;
https://github.com/astral-sh/serc/pull/568) (astral-sh/serc#569;
https://github.com/astral-sh/serc/pull/569) (astral-sh/serc#570;
https://github.com/astral-sh/serc/pull/570) (astral-sh/serc#646;
https://github.com/astral-sh/serc/pull/646) (astral-sh/serc#647;
https://github.com/astral-sh/serc/pull/647) (astral-sh/serc#666;
https://github.com/astral-sh/serc/pull/666)
- Match CPython folding limits, overflow behavior, identities, and bounds for
repeated sequences, slices, templates, bytes, and pattern keys
(astral-sh/serc#485; https://github.com/astral-sh/serc/pull/485)
(astral-sh/serc#498; https://github.com/astral-sh/serc/pull/498)
(astral-sh/serc#605; https://github.com/astral-sh/serc/pull/605)
(astral-sh/serc#606; https://github.com/astral-sh/serc/pull/606)
(astral-sh/serc#614; https://github.com/astral-sh/serc/pull/614)
(astral-sh/serc#644; https://github.com/astral-sh/serc/pull/644)
(astral-sh/serc#649; https://github.com/astral-sh/serc/pull/649)
(astral-sh/serc#652; https://github.com/astral-sh/serc/pull/652)
(astral-sh/serc#654; https://github.com/astral-sh/serc/pull/654)
(astral-sh/serc#689; https://github.com/astral-sh/serc/pull/689)
- Preserve surrogate code points and source text through string folding,
formatting, interpolation, and postponed annotations (astral-sh/serc#217;
https://github.com/astral-sh/serc/pull/217) (astral-sh/serc#219;
https://github.com/astral-sh/serc/pull/219) (astral-sh/serc#470;
https://github.com/astral-sh/serc/pull/470) (astral-sh/serc#473;
https://github.com/astral-sh/serc/pull/473) (astral-sh/serc#478;
https://github.com/astral-sh/serc/pull/478) (astral-sh/serc#648;
https://github.com/astral-sh/serc/pull/648) (astral-sh/serc#816;
https://github.com/astral-sh/serc/pull/816)
- Match CPython’s formatting of integer and large-integer postponed annotations
(astral-sh/serc#814; https://github.com/astral-sh/serc/pull/814)
(astral-sh/serc#821; https://github.com/astral-sh/serc/pull/821)

### Performance

- Compile files in parallel by default and schedule larger files first
(astral-sh/serc#603; https://github.com/astral-sh/serc/pull/603)
(astral-sh/serc#617; https://github.com/astral-sh/serc/pull/617)
- Improve compiler throughput and allocation behavior across flat functions,
constants, control flow, assembly, and marshaling (astral-sh/serc#210;
https://github.com/astral-sh/serc/pull/210) (astral-sh/serc#215;
https://github.com/astral-sh/serc/pull/215) (astral-sh/serc#232;
https://github.com/astral-sh/serc/pull/232) (astral-sh/serc#238;
https://github.com/astral-sh/serc/pull/238) (astral-sh/serc#249;
https://github.com/astral-sh/serc/pull/249) (astral-sh/serc#250;
https://github.com/astral-sh/serc/pull/250) (astral-sh/serc#256;
https://github.com/astral-sh/serc/pull/256) (astral-sh/serc#258;
https://github.com/astral-sh/serc/pull/258) (astral-sh/serc#262;
https://github.com/astral-sh/serc/pull/262) (astral-sh/serc#349;
https://github.com/astral-sh/serc/pull/349)
- Reduce repeated compiler analysis, metadata, provenance, and graph work
(astral-sh/serc#15; https://github.com/astral-sh/serc/pull/15)
(astral-sh/serc#23; https://github.com/astral-sh/serc/pull/23)
(astral-sh/serc#337; https://github.com/astral-sh/serc/pull/337)
(astral-sh/serc#345; https://github.com/astral-sh/serc/pull/345)
(astral-sh/serc#423; https://github.com/astral-sh/serc/pull/423)
(astral-sh/serc#424; https://github.com/astral-sh/serc/pull/424)
(astral-sh/serc#425; https://github.com/astral-sh/serc/pull/425)
(astral-sh/serc#580; https://github.com/astral-sh/serc/pull/580)
(astral-sh/serc#582; https://github.com/astral-sh/serc/pull/582)
(astral-sh/serc#585; https://github.com/astral-sh/serc/pull/585)
(astral-sh/serc#599; https://github.com/astral-sh/serc/pull/599)
(astral-sh/serc#618; https://github.com/astral-sh/serc/pull/618)
(astral-sh/serc#620; https://github.com/astral-sh/serc/pull/620)
(astral-sh/serc#624; https://github.com/astral-sh/serc/pull/624)
(astral-sh/serc#625; https://github.com/astral-sh/serc/pull/625)
(astral-sh/serc#626; https://github.com/astral-sh/serc/pull/626)
(astral-sh/serc#808; https://github.com/astral-sh/serc/pull/808)
- Speed up assembler passes, exception-owner replay, and bytecode and
location-table layout (astral-sh/serc#4;
https://github.com/astral-sh/serc/pull/4) (astral-sh/serc#6;
https://github.com/astral-sh/serc/pull/6) (astral-sh/serc#10;
https://github.com/astral-sh/serc/pull/10) (astral-sh/serc#14;
https://github.com/astral-sh/serc/pull/14) (astral-sh/serc#76;
https://github.com/astral-sh/serc/pull/76) (astral-sh/serc#81;
https://github.com/astral-sh/serc/pull/81) (astral-sh/serc#82;
https://github.com/astral-sh/serc/pull/82) (astral-sh/serc#83;
https://github.com/astral-sh/serc/pull/83) (astral-sh/serc#84;
https://github.com/astral-sh/serc/pull/84) (astral-sh/serc#587;
https://github.com/astral-sh/serc/pull/587) (astral-sh/serc#627;
https://github.com/astral-sh/serc/pull/627) (astral-sh/serc#628;
https://github.com/astral-sh/serc/pull/628) (astral-sh/serc#629;
https://github.com/astral-sh/serc/pull/629)
- Reduce marshal graph planning, object lookup, key allocation, and
byte-emission overhead (astral-sh/serc#5;
https://github.com/astral-sh/serc/pull/5) (astral-sh/serc#7;
https://github.com/astral-sh/serc/pull/7) (astral-sh/serc#11;
https://github.com/astral-sh/serc/pull/11) (astral-sh/serc#13;
https://github.com/astral-sh/serc/pull/13) (astral-sh/serc#77;
https://github.com/astral-sh/serc/pull/77) (astral-sh/serc#78;
https://github.com/astral-sh/serc/pull/78) (astral-sh/serc#79;
https://github.com/astral-sh/serc/pull/79) (astral-sh/serc#85;
https://github.com/astral-sh/serc/pull/85) (astral-sh/serc#86;
https://github.com/astral-sh/serc/pull/86) (astral-sh/serc#598;
https://github.com/astral-sh/serc/pull/598) (astral-sh/serc#619;
https://github.com/astral-sh/serc/pull/619) (astral-sh/serc#622;
https://github.com/astral-sh/serc/pull/622) (astral-sh/serc#670;
https://github.com/astral-sh/serc/pull/670)
- Support profile-guided release builds (astral-sh/serc#621;
https://github.com/astral-sh/serc/pull/621)

### Other changes

- Depend on published Ruff crates from crates.io for easier downstream
integration (astral-sh/serc#706; https://github.com/astral-sh/serc/pull/706)
- Refresh installation and usage documentation (astral-sh/serc#703;
https://github.com/astral-sh/serc/pull/703)
- Correct the README warning rendering and scope the documented speed claim to
the measured NumPy benchmark (astral-sh/serc#704;
https://github.com/astral-sh/serc/pull/704) (astral-sh/serc#802;
https://github.com/astral-sh/serc/pull/802)
