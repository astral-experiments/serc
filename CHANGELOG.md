# Changelog

## 0.1.8

Released on 2026-10-20.

Serc 0.1.8 speeds up fully cached compilation and validates bytecode
destinations before writing.

### Bug fixes

- Reject symlinks and other non-regular `.pyc` destinations before writing,
  including with `--force` (astral-sh/serc#1357;
  https://github.com/astral-sh/serc/pull/1357)
- Validate every `--files-from` input before writing any bytecode caches
  (astral-sh/serc#1357; https://github.com/astral-sh/serc/pull/1357)

### Performance

- Speed up fully cached compilation batches by skipping output grouping and
  compilation scheduling when every requested `.pyc` file is current
  (astral-sh/serc#1357; https://github.com/astral-sh/serc/pull/1357)

## 0.1.7

Released on 2026-10-20.

Serc 0.1.7 improves batch compilation and avoids rebuilding current bytecode
caches.

### Enhancements

- Compile explicit source-file lists with `--files-from`, using
  newline-delimited paths by default or NUL-delimited paths with `--null`,
  without scanning directories (astral-sh/serc#1355;
  https://github.com/astral-sh/serc/pull/1355)
- Skip current `.pyc` files by default when their target format, invalidation
  mode, and source fingerprint match; use `--force` to rebuild them, while
  `--check` always compiles without writing bytecode (astral-sh/serc#1355;
  https://github.com/astral-sh/serc/pull/1355)
- Report compiled and skipped file counts with `--output-format json`
  (astral-sh/serc#1355; https://github.com/astral-sh/serc/pull/1355)

## 0.1.6

Released on 2026-10-19.

Serc 0.1.6 adds standalone release binaries, improves CPython compatibility, and
reduces compilation overhead.

### Enhancements

- Download standalone Serc binaries for uv’s 18 release targets from checksummed
  archives or shell and PowerShell installers, with provenance attestations and
  a release mirror (astral-sh/serc#1352;
  https://github.com/astral-sh/serc/pull/1352)

### Bug fixes

- Preserve CPython-compatible control-flow joins and exception ownership when
  optimization removes assertions in conditionals, nested handlers, and context
  managers (astral-sh/serc#1304; https://github.com/astral-sh/serc/pull/1304)

### Performance

- Improve release-wheel and archive performance with profile-guided optimization
  on macOS ARM64, Windows x64, and GNU/Linux x64 and ARM64 (astral-sh/serc#1352;
  https://github.com/astral-sh/serc/pull/1352)
- Speed up `-O` and `-OO` compilation by reusing function scope analysis when
  assertion removal cannot change bindings or local ordering
  (astral-sh/serc#1305; https://github.com/astral-sh/serc/pull/1305)
  (astral-sh/serc#1321; https://github.com/astral-sh/serc/pull/1321)
- Reduce optimized-compilation scope analysis by retaining original scope state
  and skipping collection when no comprehension locals need reconciliation
  (astral-sh/serc#1309; https://github.com/astral-sh/serc/pull/1309)
  (astral-sh/serc#1333; https://github.com/astral-sh/serc/pull/1333)
- Speed up folding large string frozensets and reusing repeated frozenset
  constants (astral-sh/serc#1287; https://github.com/astral-sh/serc/pull/1287)
  (astral-sh/serc#1290; https://github.com/astral-sh/serc/pull/1290)
- Speed up integer-set folding by avoiding temporary numeric representations,
  including for large integers (astral-sh/serc#1292;
  https://github.com/astral-sh/serc/pull/1292) (astral-sh/serc#1297;
  https://github.com/astral-sh/serc/pull/1297)
- Speed up tuple-heavy compilation by indexing tuple-folding witnesses during
  constant remapping and operand resolution (astral-sh/serc#1332;
  https://github.com/astral-sh/serc/pull/1332) (astral-sh/serc#1334;
  https://github.com/astral-sh/serc/pull/1334)
- Reduce Python 3.12 cleanup overhead by skipping unused setup indexes and
  walking legacy `NOP` instructions directly (astral-sh/serc#1293;
  https://github.com/astral-sh/serc/pull/1293) (astral-sh/serc#1294;
  https://github.com/astral-sh/serc/pull/1294)
- Reduce legacy cleanup overhead by indexing instructions by block and avoiding
  copies when a pass makes no changes (astral-sh/serc#1301;
  https://github.com/astral-sh/serc/pull/1301) (astral-sh/serc#1339;
  https://github.com/astral-sh/serc/pull/1339)
- Speed up source validation on Python 3.12 and 3.13 by bounding prefixed-string
  scans and reusing parser tokens (astral-sh/serc#1299;
  https://github.com/astral-sh/serc/pull/1299) (astral-sh/serc#1306;
  https://github.com/astral-sh/serc/pull/1306)
- Reduce optimized-compilation time and memory use by rewriting the parsed
  syntax tree in place (astral-sh/serc#1310;
  https://github.com/astral-sh/serc/pull/1310)
- Avoid extra depth and cleanup passes for optimized expression trees
  (astral-sh/serc#1314; https://github.com/astral-sh/serc/pull/1314)
  (astral-sh/serc#1320; https://github.com/astral-sh/serc/pull/1320)
- Skip repeated constant-folding work for binary expressions and chains that
  require runtime evaluation (astral-sh/serc#1341;
  https://github.com/astral-sh/serc/pull/1341) (astral-sh/serc#1343;
  https://github.com/astral-sh/serc/pull/1343)
- Skip inapplicable sequence and tuple-folding passes on Python 3.13 and 3.15
  (astral-sh/serc#1307; https://github.com/astral-sh/serc/pull/1307)
  (astral-sh/serc#1340; https://github.com/astral-sh/serc/pull/1340)
- Speed up Python 3.15 assembly by reusing compact-layout and borrowing fast
  paths and caching opcode encodings (astral-sh/serc#1311;
  https://github.com/astral-sh/serc/pull/1311) (astral-sh/serc#1336;
  https://github.com/astral-sh/serc/pull/1336)
- Reduce assembler ownership and local-simulation overhead by avoiding
  unnecessary scans and reusing dense borrowing boundaries (astral-sh/serc#1312;
  https://github.com/astral-sh/serc/pull/1312) (astral-sh/serc#1319;
  https://github.com/astral-sh/serc/pull/1319)
- Skip redundant assembler certificate checks during `NOP` removal and unchanged
  source-location propagation (astral-sh/serc#1323;
  https://github.com/astral-sh/serc/pull/1323) (astral-sh/serc#1324;
  https://github.com/astral-sh/serc/pull/1324)
- Speed up exception-owner analysis by indexing range labels, rejecting
  irrelevant markers before provenance scans, and decoding tail operands once
  (astral-sh/serc#1330; https://github.com/astral-sh/serc/pull/1330)
  (astral-sh/serc#1331; https://github.com/astral-sh/serc/pull/1331)
  (astral-sh/serc#1337; https://github.com/astral-sh/serc/pull/1337)
- Reduce jump-layout overhead by borrowing inline fixups and reusing
  already-resolved operands (astral-sh/serc#1313;
  https://github.com/astral-sh/serc/pull/1313) (astral-sh/serc#1327;
  https://github.com/astral-sh/serc/pull/1327)
- Reduce forward-jump cleanup overhead by skipping first-pass lookups and
  deferring instruction copies (astral-sh/serc#1335;
  https://github.com/astral-sh/serc/pull/1335) (astral-sh/serc#1342;
  https://github.com/astral-sh/serc/pull/1342)
- Reduce symbol and scope-name analysis allocations by reusing parsed
  identifiers, streamlining collection, and inlining common mangling checks
  (astral-sh/serc#1317; https://github.com/astral-sh/serc/pull/1317)
  (astral-sh/serc#1326; https://github.com/astral-sh/serc/pull/1326)
  (astral-sh/serc#1329; https://github.com/astral-sh/serc/pull/1329)
- Reduce source-location lookup overhead for calls, simple assignments, and
  definitions (astral-sh/serc#1318; https://github.com/astral-sh/serc/pull/1318)
  (astral-sh/serc#1322; https://github.com/astral-sh/serc/pull/1322)
- Speed up source validation with a fast prefilter for oversized decimal
  literals (astral-sh/serc#1325; https://github.com/astral-sh/serc/pull/1325)
- Skip compiler setup for empty unreachable tails after terminating statements
  (astral-sh/serc#1328; https://github.com/astral-sh/serc/pull/1328)
- Reduce assembler memory use with compact storage for preserved jump targets
  (astral-sh/serc#1316; https://github.com/astral-sh/serc/pull/1316)
- Reduce multi-version assembly overhead by translating operands only when their
  encoding depends on the target Python version (astral-sh/serc#1295;
  https://github.com/astral-sh/serc/pull/1295)

### Other changes

- Disable explicit transparent huge-page requests in Serc’s bundled allocator
  (astral-sh/serc#1315; https://github.com/astral-sh/serc/pull/1315)

## 0.1.5

Released on 2026-10-14.

Serc 0.1.5 improves compilation performance and reduces memory use when
compiling for Python 3.12.

### Performance

- Speed up `-O` and `-OO` compilation when the syntax tree needs no changes
  (astral-sh/serc#1275; https://github.com/astral-sh/serc/pull/1275)
- Speed up compilation of functions with many sequential `try`/`except` blocks
  on Python 3.12 (astral-sh/serc#1277;
  https://github.com/astral-sh/serc/pull/1277)
- Speed up compilation of mapping patterns with many captures on Python 3.14 and
  3.15 (astral-sh/serc#1284; https://github.com/astral-sh/serc/pull/1284)
- Speed up compilation of functions with many async context managers
  (astral-sh/serc#1281; https://github.com/astral-sh/serc/pull/1281)
- Reduce source-validation overhead when targeting Python 3.12 and 3.13
  (astral-sh/serc#1279; https://github.com/astral-sh/serc/pull/1279)
- Reduce memory use when compiling for Python 3.12 (astral-sh/serc#1280;
  https://github.com/astral-sh/serc/pull/1280)
- Reduce code-object construction and copying overhead (astral-sh/serc#1262;
  https://github.com/astral-sh/serc/pull/1262) (astral-sh/serc#1263;
  https://github.com/astral-sh/serc/pull/1263) (astral-sh/serc#1265;
  https://github.com/astral-sh/serc/pull/1265)
- Reduce scope-analysis and closure-lowering overhead (astral-sh/serc#1260;
  https://github.com/astral-sh/serc/pull/1260) (astral-sh/serc#1261;
  https://github.com/astral-sh/serc/pull/1261)
- Reduce allocations during control-flow analysis (astral-sh/serc#1273;
  https://github.com/astral-sh/serc/pull/1273)
- Reduce assembler overhead for jumps, source locations, and instruction
  processing (astral-sh/serc#1268; https://github.com/astral-sh/serc/pull/1268)
  (astral-sh/serc#1269; https://github.com/astral-sh/serc/pull/1269)
  (astral-sh/serc#1266; https://github.com/astral-sh/serc/pull/1266)
  (astral-sh/serc#1267; https://github.com/astral-sh/serc/pull/1267)
  (astral-sh/serc#1270; https://github.com/astral-sh/serc/pull/1270)
  (astral-sh/serc#1258; https://github.com/astral-sh/serc/pull/1258)
- Reduce marshal serialization work (astral-sh/serc#1264;
  https://github.com/astral-sh/serc/pull/1264)
- Reduce constant-folding overhead (astral-sh/serc#1272;
  https://github.com/astral-sh/serc/pull/1272)

## 0.1.4

Released on 2026-10-12.

Serc 0.1.4 reduces compilation, assembly, and serialization overhead.

### Performance

- Reduce compilation memory use by deferring full provenance allocation for
  linear scopes (astral-sh/serc#1252;
  https://github.com/astral-sh/serc/pull/1252)
- Speed up marshal serialization by indexing the occurrence stream directly
  (astral-sh/serc#1242; https://github.com/astral-sh/serc/pull/1242)
- Reduce name and metadata allocations by reusing interned names and the
  compiler’s name index (astral-sh/serc#1231;
  https://github.com/astral-sh/serc/pull/1231) (astral-sh/serc#1235;
  https://github.com/astral-sh/serc/pull/1235)
- Reduce name-cache initialization, lookup, and memory overhead
  (astral-sh/serc#1234; https://github.com/astral-sh/serc/pull/1234)
  (astral-sh/serc#1241; https://github.com/astral-sh/serc/pull/1241)
- Speed up assembler graph processing by indexing fusion-owner boundaries and
  avoiding unnecessary cold-block and reversed-store scans (astral-sh/serc#1236;
  https://github.com/astral-sh/serc/pull/1236) (astral-sh/serc#1253;
  https://github.com/astral-sh/serc/pull/1253) (astral-sh/serc#1255;
  https://github.com/astral-sh/serc/pull/1255)
- Reduce swap optimization work by indexing instructions from the first swap
  (astral-sh/serc#1250; https://github.com/astral-sh/serc/pull/1250)
- Reduce compiler setup overhead by initializing state directly in its final
  heap allocation (astral-sh/serc#1243;
  https://github.com/astral-sh/serc/pull/1243)
- Speed up source-location and scope-exit lookups (astral-sh/serc#1251;
  https://github.com/astral-sh/serc/pull/1251) (astral-sh/serc#1256;
  https://github.com/astral-sh/serc/pull/1256)
- Skip long-decimal pre-scanning for source files too short to exceed the
  parsing limit (astral-sh/serc#1257;
  https://github.com/astral-sh/serc/pull/1257)
- Reduce small-code encoding overhead by avoiding instruction copies
  (astral-sh/serc#1254; https://github.com/astral-sh/serc/pull/1254)

## 0.1.3

Released on 2026-10-11.

Serc 0.1.3 expands CPython bytecode compatibility, especially for Python 3.14
optimized control flow, exception cleanup, and source locations.

### Bug fixes

- Match CPython loop backedges when optimization erases final coroutine-loop
  branches, including loops inside context managers (astral-sh/serc#1031;
  https://github.com/astral-sh/serc/pull/1031) (astral-sh/serc#1038;
  https://github.com/astral-sh/serc/pull/1038) (astral-sh/serc#1040;
  https://github.com/astral-sh/serc/pull/1040) (astral-sh/serc#1041;
  https://github.com/astral-sh/serc/pull/1041) (astral-sh/serc#1042;
  https://github.com/astral-sh/serc/pull/1042)
- Match CPython loop exits when optimization erases prefixes, nested-finalizer
  branches, completed manager work, or coroutine `for`/`else` arms
  (astral-sh/serc#1043; https://github.com/astral-sh/serc/pull/1043)
  (astral-sh/serc#1044; https://github.com/astral-sh/serc/pull/1044)
  (astral-sh/serc#1047; https://github.com/astral-sh/serc/pull/1047)
  (astral-sh/serc#1054; https://github.com/astral-sh/serc/pull/1054)
  (astral-sh/serc#1055; https://github.com/astral-sh/serc/pull/1055)
- Preserve optimized branch exits after erased iterator work, completed managed
  branches, and discarded constants (astral-sh/serc#1188;
  https://github.com/astral-sh/serc/pull/1188) (astral-sh/serc#1198;
  https://github.com/astral-sh/serc/pull/1198) (astral-sh/serc#1220;
  https://github.com/astral-sh/serc/pull/1220)
- Preserve loop and async-cleanup boundaries when optimization removes
  assertions or loop tails (astral-sh/serc#1102;
  https://github.com/astral-sh/serc/pull/1102) (astral-sh/serc#1104;
  https://github.com/astral-sh/serc/pull/1104) (astral-sh/serc#1107;
  https://github.com/astral-sh/serc/pull/1107) (astral-sh/serc#1114;
  https://github.com/astral-sh/serc/pull/1114)
- Preserve exits when optimization erases comprehension, context-manager,
  structured-cleanup, or finalizer work (astral-sh/serc#1106;
  https://github.com/astral-sh/serc/pull/1106) (astral-sh/serc#1108;
  https://github.com/astral-sh/serc/pull/1108) (astral-sh/serc#1112;
  https://github.com/astral-sh/serc/pull/1112) (astral-sh/serc#1118;
  https://github.com/astral-sh/serc/pull/1118)
- Preserve coroutine, loop, and finalizer exits when optimization erases
  recovery paths, assertions, or awaited alternatives (astral-sh/serc#1162;
  https://github.com/astral-sh/serc/pull/1162) (astral-sh/serc#1204;
  https://github.com/astral-sh/serc/pull/1204) (astral-sh/serc#1205;
  https://github.com/astral-sh/serc/pull/1205) (astral-sh/serc#1207;
  https://github.com/astral-sh/serc/pull/1207)
- Match CPython exception ownership when copied loop exits cross finalizers,
  backedges, or guarded handlers (astral-sh/serc#1032;
  https://github.com/astral-sh/serc/pull/1032) (astral-sh/serc#1048;
  https://github.com/astral-sh/serc/pull/1048) (astral-sh/serc#1052;
  https://github.com/astral-sh/serc/pull/1052)
- Preserve control-flow and borrowing boundaries for nested `while`/`else` loops
  and runtime loops in finalizers (astral-sh/serc#1064;
  https://github.com/astral-sh/serc/pull/1064) (astral-sh/serc#1068;
  https://github.com/astral-sh/serc/pull/1068) (astral-sh/serc#1069;
  https://github.com/astral-sh/serc/pull/1069) (astral-sh/serc#1078;
  https://github.com/astral-sh/serc/pull/1078)
- Preserve loop and retry cleanup after recovery breaks, linear `else` suites,
  and nested context managers (astral-sh/serc#1080;
  https://github.com/astral-sh/serc/pull/1080) (astral-sh/serc#1085;
  https://github.com/astral-sh/serc/pull/1085) (astral-sh/serc#1086;
  https://github.com/astral-sh/serc/pull/1086)
- Match CPython iterator-loop cleanup boundaries for chained comparisons, async
  breaks, and abrupt async `else` suites (astral-sh/serc#1046;
  https://github.com/astral-sh/serc/pull/1046) (astral-sh/serc#1049;
  https://github.com/astral-sh/serc/pull/1049) (astral-sh/serc#1088;
  https://github.com/astral-sh/serc/pull/1088) (astral-sh/serc#1089;
  https://github.com/astral-sh/serc/pull/1089)
- Preserve async-iterator exits into nested cleanup, outer-loop cleanup, and
  assertion cleanup (astral-sh/serc#1093;
  https://github.com/astral-sh/serc/pull/1093) (astral-sh/serc#1097;
  https://github.com/astral-sh/serc/pull/1097) (astral-sh/serc#1120;
  https://github.com/astral-sh/serc/pull/1120) (astral-sh/serc#1127;
  https://github.com/astral-sh/serc/pull/1127)
- Preserve iterator cleanup when runtime breaks, conditional loads, or guarded
  cleanup are involved (astral-sh/serc#1144;
  https://github.com/astral-sh/serc/pull/1144) (astral-sh/serc#1161;
  https://github.com/astral-sh/serc/pull/1161) (astral-sh/serc#1164;
  https://github.com/astral-sh/serc/pull/1164)
- Preserve natural loop and iterator exits into structured, managed, nested, and
  Boolean-guarded cleanup (astral-sh/serc#1094;
  https://github.com/astral-sh/serc/pull/1094) (astral-sh/serc#1095;
  https://github.com/astral-sh/serc/pull/1095) (astral-sh/serc#1096;
  https://github.com/astral-sh/serc/pull/1096) (astral-sh/serc#1098;
  https://github.com/astral-sh/serc/pull/1098)
- Preserve coroutine-loop recovery entries and cold continuations around nested
  cleanup and async `else` breaks (astral-sh/serc#1131;
  https://github.com/astral-sh/serc/pull/1131) (astral-sh/serc#1135;
  https://github.com/astral-sh/serc/pull/1135) (astral-sh/serc#1138;
  https://github.com/astral-sh/serc/pull/1138) (astral-sh/serc#1139;
  https://github.com/astral-sh/serc/pull/1139)
- Preserve `break` paths into conditional, guarded, and awaited cleanup
  (astral-sh/serc#1137; https://github.com/astral-sh/serc/pull/1137)
  (astral-sh/serc#1140; https://github.com/astral-sh/serc/pull/1140)
  (astral-sh/serc#1146; https://github.com/astral-sh/serc/pull/1146)
  (astral-sh/serc#1147; https://github.com/astral-sh/serc/pull/1147)
- Preserve loop and managed-recovery exits through handler cleanup, context
  managers, and runtime finalizers (astral-sh/serc#1155;
  https://github.com/astral-sh/serc/pull/1155) (astral-sh/serc#1187;
  https://github.com/astral-sh/serc/pull/1187) (astral-sh/serc#1200;
  https://github.com/astral-sh/serc/pull/1200) (astral-sh/serc#1214;
  https://github.com/astral-sh/serc/pull/1214)
- Match CPython ordering for folded breaks, coroutine loop-`else` breaks, direct
  loop exits, and finalizer entry after loops (astral-sh/serc#1202;
  https://github.com/astral-sh/serc/pull/1202) (astral-sh/serc#1203;
  https://github.com/astral-sh/serc/pull/1203) (astral-sh/serc#1212;
  https://github.com/astral-sh/serc/pull/1212) (astral-sh/serc#1217;
  https://github.com/astral-sh/serc/pull/1217) (astral-sh/serc#1223;
  https://github.com/astral-sh/serc/pull/1223)
- Preserve exception ownership when guarded finalizers and conditional
  context-manager cleanup are copied (astral-sh/serc#1035;
  https://github.com/astral-sh/serc/pull/1035) (astral-sh/serc#1036;
  https://github.com/astral-sh/serc/pull/1036) (astral-sh/serc#1050;
  https://github.com/astral-sh/serc/pull/1050) (astral-sh/serc#1053;
  https://github.com/astral-sh/serc/pull/1053)
- Preserve finalizer ownership, condition slots, and empty exits around
  terminating handlers and conditional branches (astral-sh/serc#1058;
  https://github.com/astral-sh/serc/pull/1058) (astral-sh/serc#1062;
  https://github.com/astral-sh/serc/pull/1062) (astral-sh/serc#1065;
  https://github.com/astral-sh/serc/pull/1065) (astral-sh/serc#1066;
  https://github.com/astral-sh/serc/pull/1066)
- Match CPython exception tables after erased `try`/`else` assertions and
  compacted `None` checks in nested context managers (astral-sh/serc#1072;
  https://github.com/astral-sh/serc/pull/1072) (astral-sh/serc#1087;
  https://github.com/astral-sh/serc/pull/1087)
- Preserve recovery cleanup after conditional stores, `elif` branches, and
  awaited branches in coroutines (astral-sh/serc#1057;
  https://github.com/astral-sh/serc/pull/1057) (astral-sh/serc#1059;
  https://github.com/astral-sh/serc/pull/1059) (astral-sh/serc#1063;
  https://github.com/astral-sh/serc/pull/1063)
- Preserve borrowed continuations after assertion recovery in functions,
  coroutines, and context managers (astral-sh/serc#1067;
  https://github.com/astral-sh/serc/pull/1067) (astral-sh/serc#1070;
  https://github.com/astral-sh/serc/pull/1070) (astral-sh/serc#1073;
  https://github.com/astral-sh/serc/pull/1073) (astral-sh/serc#1074;
  https://github.com/astral-sh/serc/pull/1074)
- Preserve normal cleanup after terminating, guarded, reraising, or named
  exception recovery (astral-sh/serc#1071;
  https://github.com/astral-sh/serc/pull/1071) (astral-sh/serc#1075;
  https://github.com/astral-sh/serc/pull/1075) (astral-sh/serc#1076;
  https://github.com/astral-sh/serc/pull/1076) (astral-sh/serc#1077;
  https://github.com/astral-sh/serc/pull/1077)
- Preserve normal-cleanup reachability after terminal protected bodies, nested
  guards, and guarded recovery (astral-sh/serc#1081;
  https://github.com/astral-sh/serc/pull/1081) (astral-sh/serc#1082;
  https://github.com/astral-sh/serc/pull/1082) (astral-sh/serc#1083;
  https://github.com/astral-sh/serc/pull/1083) (astral-sh/serc#1084;
  https://github.com/astral-sh/serc/pull/1084)
- Preserve nested cleanup entries after guarded raises and awaited reraises
  (astral-sh/serc#1091; https://github.com/astral-sh/serc/pull/1091)
  (astral-sh/serc#1092; https://github.com/astral-sh/serc/pull/1092)
- Preserve cleanup after awaited `try`/`else` assertions, nested recovery
  alternatives, and asserted coroutine exits (astral-sh/serc#1099;
  https://github.com/astral-sh/serc/pull/1099) (astral-sh/serc#1100;
  https://github.com/astral-sh/serc/pull/1100) (astral-sh/serc#1101;
  https://github.com/astral-sh/serc/pull/1101)
- Preserve cleanup reachability, boundaries, and ownership across abrupt
  branches and nested loops (astral-sh/serc#1109;
  https://github.com/astral-sh/serc/pull/1109) (astral-sh/serc#1110;
  https://github.com/astral-sh/serc/pull/1110) (astral-sh/serc#1111;
  https://github.com/astral-sh/serc/pull/1111) (astral-sh/serc#1113;
  https://github.com/astral-sh/serc/pull/1113)
- Preserve live cleanup continuations around iterator recovery, raising cleanup,
  completed handler values, and nested recovery (astral-sh/serc#1117;
  https://github.com/astral-sh/serc/pull/1117) (astral-sh/serc#1119;
  https://github.com/astral-sh/serc/pull/1119) (astral-sh/serc#1121;
  https://github.com/astral-sh/serc/pull/1121) (astral-sh/serc#1122;
  https://github.com/astral-sh/serc/pull/1122) (astral-sh/serc#1123;
  https://github.com/astral-sh/serc/pull/1123)
- Preserve async-manager and coroutine cleanup entries through nested,
  returning, and guarded recovery (astral-sh/serc#1116;
  https://github.com/astral-sh/serc/pull/1116) (astral-sh/serc#1124;
  https://github.com/astral-sh/serc/pull/1124) (astral-sh/serc#1125;
  https://github.com/astral-sh/serc/pull/1125) (astral-sh/serc#1126;
  https://github.com/astral-sh/serc/pull/1126)
- Preserve cleanup joins after async-manager recovery and managed async retries
  (astral-sh/serc#1130; https://github.com/astral-sh/serc/pull/1130)
  (astral-sh/serc#1133; https://github.com/astral-sh/serc/pull/1133)
  (astral-sh/serc#1136; https://github.com/astral-sh/serc/pull/1136)
- Preserve coroutine cleanup around raising guards and calls followed by nested
  `else` exits (astral-sh/serc#1141;
  https://github.com/astral-sh/serc/pull/1141) (astral-sh/serc#1149;
  https://github.com/astral-sh/serc/pull/1149)
- Preserve context-manager cleanup around outer iterators, conditional call
  arguments, and guarded async iterators (astral-sh/serc#1142;
  https://github.com/astral-sh/serc/pull/1142) (astral-sh/serc#1143;
  https://github.com/astral-sh/serc/pull/1143) (astral-sh/serc#1152;
  https://github.com/astral-sh/serc/pull/1152)
- Preserve manager cleanup after awaited or conditional stores and across
  suspension (astral-sh/serc#1151; https://github.com/astral-sh/serc/pull/1151)
  (astral-sh/serc#1154; https://github.com/astral-sh/serc/pull/1154)
  (astral-sh/serc#1157; https://github.com/astral-sh/serc/pull/1157)
  (astral-sh/serc#1168; https://github.com/astral-sh/serc/pull/1168)
- Preserve normal cleanup entries and copied recovery exits around stored calls
  and raises (astral-sh/serc#1145; https://github.com/astral-sh/serc/pull/1145)
  (astral-sh/serc#1148; https://github.com/astral-sh/serc/pull/1148)
  (astral-sh/serc#1150; https://github.com/astral-sh/serc/pull/1150)
- Preserve conditional exits and cleanup around stored reraises, guarded
  recovery, and nested repair paths (astral-sh/serc#1159;
  https://github.com/astral-sh/serc/pull/1159) (astral-sh/serc#1163;
  https://github.com/astral-sh/serc/pull/1163) (astral-sh/serc#1165;
  https://github.com/astral-sh/serc/pull/1165)
- Preserve coroutine success and recovery paths through awaited, nested, and
  `else` cleanup (astral-sh/serc#1153;
  https://github.com/astral-sh/serc/pull/1153) (astral-sh/serc#1156;
  https://github.com/astral-sh/serc/pull/1156) (astral-sh/serc#1158;
  https://github.com/astral-sh/serc/pull/1158) (astral-sh/serc#1160;
  https://github.com/astral-sh/serc/pull/1160) (astral-sh/serc#1166;
  https://github.com/astral-sh/serc/pull/1166)
- Preserve cleanup ownership after tuple unpacking, deletion, and comprehension
  work (astral-sh/serc#1115; https://github.com/astral-sh/serc/pull/1115)
  (astral-sh/serc#1128; https://github.com/astral-sh/serc/pull/1128)
  (astral-sh/serc#1132; https://github.com/astral-sh/serc/pull/1132)
  (astral-sh/serc#1134; https://github.com/astral-sh/serc/pull/1134)
  (astral-sh/serc#1167; https://github.com/astral-sh/serc/pull/1167)
- Preserve guarded and nested recovery continuations through completed work and
  finalizers (astral-sh/serc#1185; https://github.com/astral-sh/serc/pull/1185)
  (astral-sh/serc#1206; https://github.com/astral-sh/serc/pull/1206)
  (astral-sh/serc#1208; https://github.com/astral-sh/serc/pull/1208)
  (astral-sh/serc#1210; https://github.com/astral-sh/serc/pull/1210)
- Preserve copied and coroutine recovery entries into normal and nested
  finalizers (astral-sh/serc#1211; https://github.com/astral-sh/serc/pull/1211)
  (astral-sh/serc#1215; https://github.com/astral-sh/serc/pull/1215)
  (astral-sh/serc#1216; https://github.com/astral-sh/serc/pull/1216)
  (astral-sh/serc#1218; https://github.com/astral-sh/serc/pull/1218)
- Preserve finalizer entries after completed work, context-manager cleanup, and
  raising recovery (astral-sh/serc#1221;
  https://github.com/astral-sh/serc/pull/1221) (astral-sh/serc#1222;
  https://github.com/astral-sh/serc/pull/1222) (astral-sh/serc#1224;
  https://github.com/astral-sh/serc/pull/1224)
- Preserve cleanup joins and continuations across erased branches, awaited
  finalizers, and guarded coroutine recovery (astral-sh/serc#1193;
  https://github.com/astral-sh/serc/pull/1193) (astral-sh/serc#1196;
  https://github.com/astral-sh/serc/pull/1196) (astral-sh/serc#1209;
  https://github.com/astral-sh/serc/pull/1209) (astral-sh/serc#1213;
  https://github.com/astral-sh/serc/pull/1213)
- Preserve enclosing exception owners when returns copy nested finalizers
  (astral-sh/serc#1029; https://github.com/astral-sh/serc/pull/1029)
  (astral-sh/serc#1051; https://github.com/astral-sh/serc/pull/1051)
  (astral-sh/serc#1177; https://github.com/astral-sh/serc/pull/1177)
  (astral-sh/serc#1181; https://github.com/astral-sh/serc/pull/1181)
  (astral-sh/serc#1191; https://github.com/astral-sh/serc/pull/1191)
- Preserve cleanup ownership when handler returns unwind nested finalizers and
  context managers (astral-sh/serc#1061;
  https://github.com/astral-sh/serc/pull/1061) (astral-sh/serc#1178;
  https://github.com/astral-sh/serc/pull/1178) (astral-sh/serc#1225;
  https://github.com/astral-sh/serc/pull/1225) (astral-sh/serc#1226;
  https://github.com/astral-sh/serc/pull/1226)
- End inner `try` regions at the CPython-compatible point before iterator and
  context-manager cleanup (astral-sh/serc#1060;
  https://github.com/astral-sh/serc/pull/1060) (astral-sh/serc#1175;
  https://github.com/astral-sh/serc/pull/1175) (astral-sh/serc#1180;
  https://github.com/astral-sh/serc/pull/1180)
- Preserve iterator ownership and cleanup ordering through returns and
  named-handler cleanup (astral-sh/serc#1182;
  https://github.com/astral-sh/serc/pull/1182) (astral-sh/serc#1183;
  https://github.com/astral-sh/serc/pull/1183) (astral-sh/serc#1190;
  https://github.com/astral-sh/serc/pull/1190)
- Preserve shared subscript, conditional, and tuple return paths through cleanup
  (astral-sh/serc#1033; https://github.com/astral-sh/serc/pull/1033)
  (astral-sh/serc#1034; https://github.com/astral-sh/serc/pull/1034)
  (astral-sh/serc#1039; https://github.com/astral-sh/serc/pull/1039)
- Preserve return-copy paths after managed comprehensions and copied finalizers
  (astral-sh/serc#1179; https://github.com/astral-sh/serc/pull/1179)
  (astral-sh/serc#1186; https://github.com/astral-sh/serc/pull/1186)
  (astral-sh/serc#1195; https://github.com/astral-sh/serc/pull/1195)
  (astral-sh/serc#1197; https://github.com/astral-sh/serc/pull/1197)
- Preserve local-load borrowing boundaries when returns cross handler,
  finalizer, or branching call cleanup (astral-sh/serc#1030;
  https://github.com/astral-sh/serc/pull/1030) (astral-sh/serc#1037;
  https://github.com/astral-sh/serc/pull/1037) (astral-sh/serc#1045;
  https://github.com/astral-sh/serc/pull/1045) (astral-sh/serc#1090;
  https://github.com/astral-sh/serc/pull/1090)
- Match CPython constant folding and local-load fusion after copied exits and
  exception handlers (astral-sh/serc#1171;
  https://github.com/astral-sh/serc/pull/1171) (astral-sh/serc#1174;
  https://github.com/astral-sh/serc/pull/1174) (astral-sh/serc#1176;
  https://github.com/astral-sh/serc/pull/1176) (astral-sh/serc#1184;
  https://github.com/astral-sh/serc/pull/1184)
- Match CPython cleanup rotations and boundaries for sequence and mapping
  patterns (astral-sh/serc#1172; https://github.com/astral-sh/serc/pull/1172)
  (astral-sh/serc#1173; https://github.com/astral-sh/serc/pull/1173)
  (astral-sh/serc#1192; https://github.com/astral-sh/serc/pull/1192)
- Preserve module cell ordering when optimization removes assertions
  (astral-sh/serc#1169; https://github.com/astral-sh/serc/pull/1169)
- Preserve interrupt and monitoring checks through async-match exhaustion and
  handler cleanup (astral-sh/serc#1170;
  https://github.com/astral-sh/serc/pull/1170) (astral-sh/serc#1194;
  https://github.com/astral-sh/serc/pull/1194) (astral-sh/serc#1232;
  https://github.com/astral-sh/serc/pull/1232)
- Match CPython bytecode for terminal conditional name stores
  (astral-sh/serc#1201; https://github.com/astral-sh/serc/pull/1201)
- Preserve return source locations after removed assertions and completed
  prefixes (astral-sh/serc#1103; https://github.com/astral-sh/serc/pull/1103)
  (astral-sh/serc#1228; https://github.com/astral-sh/serc/pull/1228)
  (astral-sh/serc#1229; https://github.com/astral-sh/serc/pull/1229)
  (astral-sh/serc#1230; https://github.com/astral-sh/serc/pull/1230)
- Preserve return source locations after managed branches, erased coroutine
  assertions, and async cleanup (astral-sh/serc#1238;
  https://github.com/astral-sh/serc/pull/1238) (astral-sh/serc#1239;
  https://github.com/astral-sh/serc/pull/1239) (astral-sh/serc#1240;
  https://github.com/astral-sh/serc/pull/1240)
- Preserve authored break lines, unlocated manager joins, and line markers
  through nested finalizers (astral-sh/serc#1079;
  https://github.com/astral-sh/serc/pull/1079) (astral-sh/serc#1105;
  https://github.com/astral-sh/serc/pull/1105) (astral-sh/serc#1129;
  https://github.com/astral-sh/serc/pull/1129)
- Preserve original guard, exit, scope-footer, and copied-match source locations
  (astral-sh/serc#1219; https://github.com/astral-sh/serc/pull/1219)
  (astral-sh/serc#1227; https://github.com/astral-sh/serc/pull/1227)
  (astral-sh/serc#1233; https://github.com/astral-sh/serc/pull/1233)
  (astral-sh/serc#1237; https://github.com/astral-sh/serc/pull/1237)
- Avoid a debug-build assertion failure when legacy return blocks receive a late
  leading source location (astral-sh/serc#1056;
  https://github.com/astral-sh/serc/pull/1056)

### Other changes

- Focus the compatibility documentation on selecting and reproducing supported
  reference builds (astral-sh/serc#1244;
  https://github.com/astral-sh/serc/pull/1244)

## 0.1.2

Released on 2026-10-05.

Serc 0.1.2 improves CPython bytecode compatibility and speeds up compilation and
serialization.

### Bug fixes

- Preserve shared returns and cleanup exits when constant-true assertions are
  folded on Python 3.12 (astral-sh/serc#975;
  https://github.com/astral-sh/serc/pull/975)
- Match CPython control flow when optimization removes nested assertions or
  branch prefixes (astral-sh/serc#989;
  https://github.com/astral-sh/serc/pull/989) (astral-sh/serc#991;
  https://github.com/astral-sh/serc/pull/991) (astral-sh/serc#998;
  https://github.com/astral-sh/serc/pull/998) (astral-sh/serc#1005;
  https://github.com/astral-sh/serc/pull/1005) (astral-sh/serc#1027;
  https://github.com/astral-sh/serc/pull/1027)
- Match CPython control flow in coroutines and nested iterators after assertion
  removal (astral-sh/serc#995; https://github.com/astral-sh/serc/pull/995)
  (astral-sh/serc#1016; https://github.com/astral-sh/serc/pull/1016)
- Preserve implicit returns after optimization erases terminal loop assertions
  (astral-sh/serc#1012; https://github.com/astral-sh/serc/pull/1012)
- Match CPython loop bytecode when optimization removes `else` assertions after
  assignments, comprehensions, and inner loops (astral-sh/serc#1004;
  https://github.com/astral-sh/serc/pull/1004) (astral-sh/serc#1006;
  https://github.com/astral-sh/serc/pull/1006) (astral-sh/serc#1007;
  https://github.com/astral-sh/serc/pull/1007) (astral-sh/serc#1008;
  https://github.com/astral-sh/serc/pull/1008)
- Match CPython loop bytecode after assertion removal inside `try` blocks and
  context managers (astral-sh/serc#993;
  https://github.com/astral-sh/serc/pull/993) (astral-sh/serc#999;
  https://github.com/astral-sh/serc/pull/999) (astral-sh/serc#1019;
  https://github.com/astral-sh/serc/pull/1019) (astral-sh/serc#1020;
  https://github.com/astral-sh/serc/pull/1020)
- Match CPython loop bytecode in coroutines and async loops after awaits and
  assertion removal (astral-sh/serc#994;
  https://github.com/astral-sh/serc/pull/994) (astral-sh/serc#1002;
  https://github.com/astral-sh/serc/pull/1002) (astral-sh/serc#1010;
  https://github.com/astral-sh/serc/pull/1010) (astral-sh/serc#1011;
  https://github.com/astral-sh/serc/pull/1011)
- Match CPython cleanup ordering when `break` or `continue` exits through
  `finally` blocks (astral-sh/serc#977;
  https://github.com/astral-sh/serc/pull/977) (astral-sh/serc#981;
  https://github.com/astral-sh/serc/pull/981) (astral-sh/serc#1022;
  https://github.com/astral-sh/serc/pull/1022)
- Match CPython exception tables for returns and conditional cleanup in
  `finally` blocks (astral-sh/serc#982;
  https://github.com/astral-sh/serc/pull/982) (astral-sh/serc#983;
  https://github.com/astral-sh/serc/pull/983) (astral-sh/serc#984;
  https://github.com/astral-sh/serc/pull/984)
- Match CPython exception tables for returns and nested cleanup in async context
  managers (astral-sh/serc#985; https://github.com/astral-sh/serc/pull/985)
- Match CPython iterator cleanup when `finally` blocks call methods or local
  callables (astral-sh/serc#986; https://github.com/astral-sh/serc/pull/986)
  (astral-sh/serc#992; https://github.com/astral-sh/serc/pull/992)
- Match CPython local-variable loads after conditional cleanup, `try`/`else`
  breaks, and exception-handler exits (astral-sh/serc#976;
  https://github.com/astral-sh/serc/pull/976) (astral-sh/serc#1000;
  https://github.com/astral-sh/serc/pull/1000) (astral-sh/serc#1003;
  https://github.com/astral-sh/serc/pull/1003) (astral-sh/serc#1009;
  https://github.com/astral-sh/serc/pull/1009)
- Match CPython local-variable loads after conditional loops in `finally` blocks
  (astral-sh/serc#1001; https://github.com/astral-sh/serc/pull/1001)
- Match CPython exception tables for conditional tuple returns inside `try`
  blocks (astral-sh/serc#1015; https://github.com/astral-sh/serc/pull/1015)
- Match CPython bytecode and source locations for exception-handler cleanup and
  closure setup (astral-sh/serc#979; https://github.com/astral-sh/serc/pull/979)
  (astral-sh/serc#980; https://github.com/astral-sh/serc/pull/980)
- Match CPython exception tables when `break` or `return` exits nested exception
  handlers and `finally` blocks (astral-sh/serc#1023;
  https://github.com/astral-sh/serc/pull/1023) (astral-sh/serc#1024;
  https://github.com/astral-sh/serc/pull/1024) (astral-sh/serc#1025;
  https://github.com/astral-sh/serc/pull/1025) (astral-sh/serc#1026;
  https://github.com/astral-sh/serc/pull/1026)
- Match CPython cleanup for sequence patterns with multiple wildcards and match
  statements inside unnamed exception handlers (astral-sh/serc#1018;
  https://github.com/astral-sh/serc/pull/1018) (astral-sh/serc#1021;
  https://github.com/astral-sh/serc/pull/1021)
- Keep later global references independent from temporary comprehension targets
  (astral-sh/serc#972; https://github.com/astral-sh/serc/pull/972)
- Calculate the correct stack size for later `except*` handlers containing
  calls, suspension, or nested cleanup on Python 3.14 and 3.15
  (astral-sh/serc#974; https://github.com/astral-sh/serc/pull/974)
- Compile deeply nested statement suites without overflowing small thread stacks
  (astral-sh/serc#971; https://github.com/astral-sh/serc/pull/971)
- Preserve the final operand’s source line in chained `None` comparisons on
  Python 3.12 and 3.13 (astral-sh/serc#973;
  https://github.com/astral-sh/serc/pull/973)
- Match CPython startup string interning and marshal identity on Linux for the
  documented native reference profile across Python 3.12 through 3.15
  (astral-sh/serc#978; https://github.com/astral-sh/serc/pull/978)
- Match CPython constant tables and bytecode when folding Boolean expressions
  and starred collection literals (astral-sh/serc#987;
  https://github.com/astral-sh/serc/pull/987) (astral-sh/serc#988;
  https://github.com/astral-sh/serc/pull/988)

### Performance

- Speed up marshal serialization (astral-sh/serc#930;
  https://github.com/astral-sh/serc/pull/930) (astral-sh/serc#931;
  https://github.com/astral-sh/serc/pull/931) (astral-sh/serc#932;
  https://github.com/astral-sh/serc/pull/932) (astral-sh/serc#962;
  https://github.com/astral-sh/serc/pull/962)
- Reduce compilation overhead (astral-sh/serc#948;
  https://github.com/astral-sh/serc/pull/948) (astral-sh/serc#964;
  https://github.com/astral-sh/serc/pull/964)
- Speed up compilation of annotation-heavy code (astral-sh/serc#996;
  https://github.com/astral-sh/serc/pull/996) (astral-sh/serc#997;
  https://github.com/astral-sh/serc/pull/997) (astral-sh/serc#1013;
  https://github.com/astral-sh/serc/pull/1013)
- Reduce memory allocations during compilation (astral-sh/serc#1017;
  https://github.com/astral-sh/serc/pull/1017)
- Reduce repeated work when compiling folded constants (astral-sh/serc#1014;
  https://github.com/astral-sh/serc/pull/1014)
- Speed up source nesting validation (astral-sh/serc#930;
  https://github.com/astral-sh/serc/pull/930) (astral-sh/serc#931;
  https://github.com/astral-sh/serc/pull/931) (astral-sh/serc#932;
  https://github.com/astral-sh/serc/pull/932)

## 0.1.1

Released on 2026-09-29.

Serc 0.1.1 adds Python optimization modes and hash-based bytecode caches,
improves compilation performance, and expands compatibility with CPython.

### Enhancements

- Compile with Python’s `-O` and `-OO` optimization modes through CLI flags or
  `CompileOptions` (astral-sh/serc#841;
  https://github.com/astral-sh/serc/pull/841)
- Generate checked or unchecked hash-based bytecode caches with
  `--invalidation-mode` or `CompiledModule::to_hash_pyc` (astral-sh/serc#840;
  https://github.com/astral-sh/serc/pull/840)
- Opt into caller-selected decimal literal parsing limits through new text,
  byte, and path compilation APIs (astral-sh/serc#911;
  https://github.com/astral-sh/serc/pull/911)
- Model the caller’s integer-to-string limit when formatting duplicate
  mapping-key diagnostics through opt-in compilation APIs (astral-sh/serc#887;
  https://github.com/astral-sh/serc/pull/887)
- Reproduce floating-point constant folding from documented CPython 3.14.5 GCC
  and Clang builds on supported GNU/Linux x86-64 hosts by selecting a native
  build profile (astral-sh/serc#824; https://github.com/astral-sh/serc/pull/824)

### Bug fixes

- Produce CPython-compatible control flow, source locations, exception tables,
  and constant sharing under `-O` and `-OO` (astral-sh/serc#912;
  https://github.com/astral-sh/serc/pull/912)
- Preserve requested `.opt-N.pyc` cache suffixes for repeated optimization flags
  while applying `-OO` semantics (astral-sh/serc#912;
  https://github.com/astral-sh/serc/pull/912)
- Compile nested `continue` statements in `finally` suites without negative
  stack-depth errors or incorrect exception restoration (astral-sh/serc#825;
  https://github.com/astral-sh/serc/pull/825)
- Compile deeply nested expressions and postponed annotations without
  overflowing the compiler stack (astral-sh/serc#827;
  https://github.com/astral-sh/serc/pull/827)
- Honor selected native build profiles throughout nested constant folding
  (astral-sh/serc#827; https://github.com/astral-sh/serc/pull/827)
- Keep enclosing cleanup and exception handlers active when tracing raises at a
  nested no-op `finally` suite (astral-sh/serc#900;
  https://github.com/astral-sh/serc/pull/900) (astral-sh/serc#908;
  https://github.com/astral-sh/serc/pull/908)
- Convert `StopIteration` raised by tracing in nested generator finalizers into
  `RuntimeError` instead of silently terminating the generator
  (astral-sh/serc#903; https://github.com/astral-sh/serc/pull/903)
- Resolve private type-parameter names and their mangled aliases to the same
  binding in generic class headers (astral-sh/serc#828;
  https://github.com/astral-sh/serc/pull/828)
- Preserve marshal object identity for retained native startup names on Python
  3.14 (astral-sh/serc#830; https://github.com/astral-sh/serc/pull/830)
- Remove redundant `NOP` instructions before module annotations after literal
  and negated conditions on Python 3.14 and 3.15 (astral-sh/serc#894;
  https://github.com/astral-sh/serc/pull/894) (astral-sh/serc#906;
  https://github.com/astral-sh/serc/pull/906)
- Match CPython opcode ordering when breaking from an exceptional `finally`
  suite into nested handler cleanup (astral-sh/serc#829;
  https://github.com/astral-sh/serc/pull/829)
- Preserve async-comprehension assertion exits in terminal conditionals on
  Python 3.12 (astral-sh/serc#889; https://github.com/astral-sh/serc/pull/889)
- Match CPython’s exception-table coverage for markers in nested finalizers
  after a handler returns (astral-sh/serc#904;
  https://github.com/astral-sh/serc/pull/904)
- Match CPython bytecode and exception-table ownership after branching `try`
  bodies enter nested finalizers (astral-sh/serc#909;
  https://github.com/astral-sh/serc/pull/909)
- Match CPython 3.12 source locations for successful assertions at the end of
  `finally` suites (astral-sh/serc#907;
  https://github.com/astral-sh/serc/pull/907)
- Reject lambdas and comprehensions in Python 3.12 generic headers when their
  annotation scopes can see an enclosing class namespace (astral-sh/serc#831;
  https://github.com/astral-sh/serc/pull/831)
- Reject repeated explicit keyword arguments in calls, including unreachable
  code (astral-sh/serc#845; https://github.com/astral-sh/serc/pull/845)
- Reject invalid uses of `__debug__` in attributes, call and class keywords, and
  class patterns according to the target Python version (astral-sh/serc#847;
  https://github.com/astral-sh/serc/pull/847)
- Reject commas immediately following bare `yield` expressions while continuing
  to accept parenthesized yields and yield tuples (astral-sh/serc#846;
  https://github.com/astral-sh/serc/pull/846)
- Reject Python sources with more than 99 active indentation levels
  (astral-sh/serc#834; https://github.com/astral-sh/serc/pull/834)
- Enforce CPython’s default 4,300-digit limit for decimal integer literals
  (astral-sh/serc#835; https://github.com/astral-sh/serc/pull/835)
- Reject parameter specification and variadic type-parameter defaults when
  targeting Python 3.12 (astral-sh/serc#869;
  https://github.com/astral-sh/serc/pull/869) (astral-sh/serc#870;
  https://github.com/astral-sh/serc/pull/870)
- Include the repeated type-parameter name in duplicate-name diagnostics
  (astral-sh/serc#850; https://github.com/astral-sh/serc/pull/850)
- Match CPython diagnostics for unknown `__future__` features
  (astral-sh/serc#858; https://github.com/astral-sh/serc/pull/858)
- Match CPython diagnostics for misplaced `__future__` imports
  (astral-sh/serc#860; https://github.com/astral-sh/serc/pull/860)
- Prioritize misplaced `__future__` imports over unknown features in the same
  statement (astral-sh/serc#865; https://github.com/astral-sh/serc/pull/865)
- Match CPython diagnostics and source ranges for duplicate mapping-pattern keys
  (astral-sh/serc#857; https://github.com/astral-sh/serc/pull/857)
- Reject large integer mapping-pattern keys that compare equal to floating-point
  or complex keys (astral-sh/serc#861;
  https://github.com/astral-sh/serc/pull/861)
- Preserve lone surrogates when validating and folding mapping-pattern keys
  (astral-sh/serc#877; https://github.com/astral-sh/serc/pull/877)
- Match CPython diagnostics for assignment expressions targeting attributes
  (astral-sh/serc#871; https://github.com/astral-sh/serc/pull/871)
- Match CPython diagnostics for unparenthesized tuples after `yield from`
  (astral-sh/serc#872; https://github.com/astral-sh/serc/pull/872)
- Preserve type-alias default diagnostics when later syntax errors are also
  present (astral-sh/serc#867; https://github.com/astral-sh/serc/pull/867)
- Report the appropriate comprehension-specific error for `yield` and
  `yield from` expressions inside comprehensions (astral-sh/serc#879;
  https://github.com/astral-sh/serc/pull/879)
- Report direct dictionary-comprehension value `yield` errors before
  corresponding key errors (astral-sh/serc#880;
  https://github.com/astral-sh/serc/pull/880)
- Report nested comprehension `yield` errors in operand order
  (astral-sh/serc#884; https://github.com/astral-sh/serc/pull/884)
- Prioritize comprehension `yield` errors over earlier bare starred expressions
  (astral-sh/serc#885; https://github.com/astral-sh/serc/pull/885)
- Prioritize supported comprehension `yield` errors over async-generator
  value-return errors (astral-sh/serc#895;
  https://github.com/astral-sh/serc/pull/895)
- Match CPython diagnostics for `await` inside synchronous functions and lambdas
  (astral-sh/serc#896; https://github.com/astral-sh/serc/pull/896)
- Resolve invalid `global` and `nonlocal` declarations before reporting
  mapping-pattern errors (astral-sh/serc#881;
  https://github.com/astral-sh/serc/pull/881)

### Performance

- Reduce compilation overhead across scope analysis, source handling, assembler
  passes, and marshal serialization (astral-sh/serc#848;
  https://github.com/astral-sh/serc/pull/848)

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
