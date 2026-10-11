# Changelog

## 0.1.3

Released on 2026-10-11.

### Other changes

- Allow Linux tests more time to finish ([#1199](https://github.com/astral-sh/serc/pull/1199))
- Allow late leading locations in legacy return blocks ([#1056](https://github.com/astral-sh/serc/pull/1056))
- Centralize assembler instruction accessors ([#1245](https://github.com/astral-sh/serc/pull/1245))
- Copy direct exits after discarded constants ([#1220](https://github.com/astral-sh/serc/pull/1220))
- Fold sequence arithmetic exposed by copied exits ([#1171](https://github.com/astral-sh/serc/pull/1171))
- Fuse conditional attribute copies across aliased entries ([#1174](https://github.com/astral-sh/serc/pull/1174))
- Fuse protected tuple locals with matching scope fallbacks ([#1184](https://github.com/astral-sh/serc/pull/1184))
- Fuse unprotected local pairs after exception handlers ([#1176](https://github.com/astral-sh/serc/pull/1176))
- Improve handler cleanup reachability ([#1109](https://github.com/astral-sh/serc/pull/1109))
- Inline stored reraises before revisiting conditional exits ([#1159](https://github.com/astral-sh/serc/pull/1159))
- Keep erased-loop manager cleanup joins unlocated ([#1105](https://github.com/astral-sh/serc/pull/1105))
- Keep nested repair cleanup behind its empty normal entry ([#1165](https://github.com/astral-sh/serc/pull/1165))
- Preserve asserted coroutine exits into awaited cleanup ([#1101](https://github.com/astral-sh/serc/pull/1101))
- Preserve asserted recovery across enclosing manager exits ([#1074](https://github.com/astral-sh/serc/pull/1074))
- Preserve asserted success exits after awaited stores ([#1153](https://github.com/astral-sh/serc/pull/1153))
- Preserve async break boundaries after completed comparisons ([#1089](https://github.com/astral-sh/serc/pull/1089))
- Preserve async iterator breaks into outer-loop cleanup ([#1097](https://github.com/astral-sh/serc/pull/1097))
- Preserve awaited else-break cleanup entries ([#1147](https://github.com/astral-sh/serc/pull/1147))
- Preserve awaited entries into nested cleanup ([#1124](https://github.com/astral-sh/serc/pull/1124))
- Preserve awaited finalizer continuation ownership ([#1209](https://github.com/astral-sh/serc/pull/1209))
- Preserve awaited loop exits into nested cleanup ([#1093](https://github.com/astral-sh/serc/pull/1093))
- Preserve awaited re-raise cleanup entries ([#1092](https://github.com/astral-sh/serc/pull/1092))
- Preserve awaited store entries through managed recovery ([#1151](https://github.com/astral-sh/serc/pull/1151))
- Preserve borrow boundaries through return cleanup ([#1030](https://github.com/astral-sh/serc/pull/1030))
- Preserve borrowed continuations after asserted coroutine recovery ([#1067](https://github.com/astral-sh/serc/pull/1067))
- Preserve borrowed continuations after asserted recovery ([#1070](https://github.com/astral-sh/serc/pull/1070))
- Preserve borrowed continuations after runtime recovery assertions ([#1073](https://github.com/astral-sh/serc/pull/1073))
- Preserve borrowed loads in conditional iterator arms ([#1161](https://github.com/astral-sh/serc/pull/1161))
- Preserve borrowed loads through chained comparison cleanup ([#1046](https://github.com/astral-sh/serc/pull/1046))
- Preserve borrowing boundaries after asynchronous loop else suites ([#1049](https://github.com/astral-sh/serc/pull/1049))
- Preserve borrowing boundaries after terminating recovery else suites ([#1071](https://github.com/astral-sh/serc/pull/1071))
- Preserve cleanup after completed deletion ([#1128](https://github.com/astral-sh/serc/pull/1128))
- Preserve cleanup after completed handler values ([#1121](https://github.com/astral-sh/serc/pull/1121))
- Preserve cleanup boundaries after asserted iterator breaks ([#1120](https://github.com/astral-sh/serc/pull/1120))
- Preserve cleanup boundaries after erased loop tails ([#1114](https://github.com/astral-sh/serc/pull/1114))
- Preserve cleanup boundaries in asynchronous managers ([#1116](https://github.com/astral-sh/serc/pull/1116))
- Preserve cleanup boundaries with prefixed work ([#1111](https://github.com/astral-sh/serc/pull/1111))
- Preserve cleanup entries across abrupt branches ([#1110](https://github.com/astral-sh/serc/pull/1110))
- Preserve cleanup entries after comprehension recovery ([#1132](https://github.com/astral-sh/serc/pull/1132))
- Preserve cleanup entry after completed comprehension stores ([#1167](https://github.com/astral-sh/serc/pull/1167))
- Preserve cleanup exits after conditional manager stores ([#1154](https://github.com/astral-sh/serc/pull/1154))
- Preserve cleanup joins after managed async retries ([#1136](https://github.com/astral-sh/serc/pull/1136))
- Preserve cleanup owners through negated finally guards ([#1035](https://github.com/astral-sh/serc/pull/1035))
- Preserve cleanup ownership across nested loops ([#1113](https://github.com/astral-sh/serc/pull/1113))
- Preserve cleanup ownership through nested handler returns ([#1178](https://github.com/astral-sh/serc/pull/1178))
- Preserve cleanup reachability after completed nested guards ([#1083](https://github.com/astral-sh/serc/pull/1083))
- Preserve cleanup reachability inside nested managers ([#1085](https://github.com/astral-sh/serc/pull/1085))
- Preserve cold continuations after async else breaks ([#1139](https://github.com/astral-sh/serc/pull/1139))
- Preserve cold continuations after async manager recovery ([#1130](https://github.com/astral-sh/serc/pull/1130))
- Preserve cold continuations after guarded loop recovery ([#1135](https://github.com/astral-sh/serc/pull/1135))
- Preserve completed loop work through Boolean cleanup ([#1098](https://github.com/astral-sh/serc/pull/1098))
- Preserve completed managed iterator branch tails ([#1198](https://github.com/astral-sh/serc/pull/1198))
- Preserve composed mapping capture cleanup swaps ([#1173](https://github.com/astral-sh/serc/pull/1173))
- Preserve comprehension return locations after completed prefixes ([#1229](https://github.com/astral-sh/serc/pull/1229))
- Preserve conditional cleanup after finalizer prefixes ([#1036](https://github.com/astral-sh/serc/pull/1036))
- Preserve conditional continue exits after erased prefixes ([#1043](https://github.com/astral-sh/serc/pull/1043))
- Preserve conditional entries into normal finalizers ([#1216](https://github.com/astral-sh/serc/pull/1216))
- Preserve conditional iterator-return cleanup copies ([#1034](https://github.com/astral-sh/serc/pull/1034))
- Preserve copied finalizer entry slots after managed returns ([#1186](https://github.com/astral-sh/serc/pull/1186))
- Preserve copied recovery exits after completed cleanup ([#1150](https://github.com/astral-sh/serc/pull/1150))
- Preserve copied recovery exits before outer cleanup ([#1211](https://github.com/astral-sh/serc/pull/1211))
- Preserve coroutine branch return locations after erased assertions ([#1239](https://github.com/astral-sh/serc/pull/1239))
- Preserve coroutine call entries through nested else exits ([#1149](https://github.com/astral-sh/serc/pull/1149))
- Preserve coroutine cleanup joins before awaited successors ([#1193](https://github.com/astral-sh/serc/pull/1193))
- Preserve coroutine for-else branch exits ([#1055](https://github.com/astral-sh/serc/pull/1055))
- Preserve coroutine loop entries into nested cleanup ([#1131](https://github.com/astral-sh/serc/pull/1131))
- Preserve coroutine loop exits through conditional recovery ([#1155](https://github.com/astral-sh/serc/pull/1155))
- Preserve coroutine loop exits through erased assertion tails ([#1204](https://github.com/astral-sh/serc/pull/1204))
- Preserve coroutine loop-else break ordering ([#1203](https://github.com/astral-sh/serc/pull/1203))
- Preserve coroutine raising-guard cleanup entries ([#1141](https://github.com/astral-sh/serc/pull/1141))
- Preserve coroutine recovery borrowing after awaited branches ([#1063](https://github.com/astral-sh/serc/pull/1063))
- Preserve coroutine recovery borrowing after conditional stores ([#1059](https://github.com/astral-sh/serc/pull/1059))
- Preserve coroutine recovery entries before cleanup ([#1215](https://github.com/astral-sh/serc/pull/1215))
- Preserve coroutine return locations after erased prefixes ([#1230](https://github.com/astral-sh/serc/pull/1230))
- Preserve direct break entries past erased recovery exits ([#1162](https://github.com/astral-sh/serc/pull/1162))
- Preserve direct loop exits into try-else continuations ([#1212](https://github.com/astral-sh/serc/pull/1212))
- Preserve empty async iterator exits before abrupt else suites ([#1088](https://github.com/astral-sh/serc/pull/1088))
- Preserve empty async-iterator cleanup entries ([#1127](https://github.com/astral-sh/serc/pull/1127))
- Preserve empty cleanup exits after recovery breaks ([#1080](https://github.com/astral-sh/serc/pull/1080))
- Preserve empty entries before awaited coroutine cleanup ([#1158](https://github.com/astral-sh/serc/pull/1158))
- Preserve empty entries left by copied raises ([#1148](https://github.com/astral-sh/serc/pull/1148))
- Preserve empty finalizer exits before manager cleanup ([#1066](https://github.com/astral-sh/serc/pull/1066))
- Preserve empty joins before recovery cleanup ([#1133](https://github.com/astral-sh/serc/pull/1133))
- Preserve empty loop-tail try exception owners ([#1065](https://github.com/astral-sh/serc/pull/1065))
- Preserve empty recovery exits after terminal protected bodies ([#1082](https://github.com/astral-sh/serc/pull/1082))
- Preserve empty retry exits after linear else suites ([#1086](https://github.com/astral-sh/serc/pull/1086))
- Preserve enclosing finalizers around loop continues ([#1032](https://github.com/astral-sh/serc/pull/1032))
- Preserve enclosing manager ownership through guarded recovery ([#1050](https://github.com/astral-sh/serc/pull/1050))
- Preserve enclosing try owners through named handler returns ([#1029](https://github.com/astral-sh/serc/pull/1029))
- Preserve enclosing try owners through return cleanup ([#1191](https://github.com/astral-sh/serc/pull/1191))
- Preserve enclosing try ownership in copied finalizers ([#1177](https://github.com/astral-sh/serc/pull/1177))
- Preserve enclosing try ownership until handler finalizers start ([#1181](https://github.com/astral-sh/serc/pull/1181))
- Preserve erased async cleanup after comprehension stores ([#1106](https://github.com/astral-sh/serc/pull/1106))
- Preserve erased async finalizer borrow boundaries ([#1102](https://github.com/astral-sh/serc/pull/1102))
- Preserve erased exits after structured cleanup ([#1112](https://github.com/astral-sh/serc/pull/1112))
- Preserve erased finalizer exits after awaited alternatives ([#1207](https://github.com/astral-sh/serc/pull/1207))
- Preserve erased loop branches inside nested finalizers ([#1044](https://github.com/astral-sh/serc/pull/1044))
- Preserve erased manager cleanup exits ([#1108](https://github.com/astral-sh/serc/pull/1108))
- Preserve erased while-tail borrow boundaries ([#1107](https://github.com/astral-sh/serc/pull/1107))
- Preserve exception boundaries on copied backedges ([#1048](https://github.com/astral-sh/serc/pull/1048))
- Preserve exception ownership across copied finalizers ([#1051](https://github.com/astral-sh/serc/pull/1051))
- Preserve exception ownership after erased try-else assertions ([#1072](https://github.com/astral-sh/serc/pull/1072))
- Preserve exception ownership for conditional exits before finalizers ([#1058](https://github.com/astral-sh/serc/pull/1058))
- Preserve exits after iterator recovery ([#1117](https://github.com/astral-sh/serc/pull/1117))
- Preserve finalizer condition slots after terminating handlers ([#1062](https://github.com/astral-sh/serc/pull/1062))
- Preserve finalizer recovery continuation ownership ([#1208](https://github.com/astral-sh/serc/pull/1208))
- Preserve finally borrows after conditional handler returns ([#1037](https://github.com/astral-sh/serc/pull/1037))
- Preserve folded loop breaks through handler cleanup ([#1202](https://github.com/astral-sh/serc/pull/1202))
- Preserve guard locations on copied match exits ([#1237](https://github.com/astral-sh/serc/pull/1237))
- Preserve guarded break cleanup ownership ([#1146](https://github.com/astral-sh/serc/pull/1146))
- Preserve guarded cleanup entries in managed coroutines ([#1126](https://github.com/astral-sh/serc/pull/1126))
- Preserve guarded coroutine recovery into awaited cleanup ([#1213](https://github.com/astral-sh/serc/pull/1213))
- Preserve guarded coroutine recovery into nested cleanup ([#1160](https://github.com/astral-sh/serc/pull/1160))
- Preserve guarded iterator cleanup under async managers ([#1152](https://github.com/astral-sh/serc/pull/1152))
- Preserve guarded recovery continuations in normal finally bodies ([#1084](https://github.com/astral-sh/serc/pull/1084))
- Preserve guarded recovery exits through completed work ([#1163](https://github.com/astral-sh/serc/pull/1163))
- Preserve guarded recovery exits through completed work ([#1206](https://github.com/astral-sh/serc/pull/1206))
- Preserve guarded recovery through manager cleanup ([#1222](https://github.com/astral-sh/serc/pull/1222))
- Preserve handler entries around nested finalizers ([#1218](https://github.com/astral-sh/serc/pull/1218))
- Preserve interrupt checks after async match exhaustion ([#1170](https://github.com/astral-sh/serc/pull/1170))
- Preserve interrupt checks through named handler break cleanup ([#1194](https://github.com/astral-sh/serc/pull/1194))
- Preserve iterator branch exits around erased work ([#1047](https://github.com/astral-sh/serc/pull/1047))
- Preserve iterator break entry into guarded group cleanup ([#1164](https://github.com/astral-sh/serc/pull/1164))
- Preserve iterator cleanup after erased assertions ([#1104](https://github.com/astral-sh/serc/pull/1104))
- Preserve iterator cleanup after guarded handler joins ([#1052](https://github.com/astral-sh/serc/pull/1052))
- Preserve iterator exhaustion through structured cleanup ([#1094](https://github.com/astral-sh/serc/pull/1094))
- Preserve iterator exits inside copied finalizers ([#1197](https://github.com/astral-sh/serc/pull/1197))
- Preserve iterator return locations after erased assertions ([#1103](https://github.com/astral-sh/serc/pull/1103))
- Preserve iterator return owners and completed finalizer locations ([#1182](https://github.com/astral-sh/serc/pull/1182))
- Preserve line markers through nested finalizers ([#1129](https://github.com/astral-sh/serc/pull/1129))
- Preserve live cleanup spans for managed handler returns ([#1226](https://github.com/astral-sh/serc/pull/1226))
- Preserve live continuations across erased cleanup branches ([#1196](https://github.com/astral-sh/serc/pull/1196))
- Preserve live else exits after nested recovery ([#1123](https://github.com/astral-sh/serc/pull/1123))
- Preserve live entries through raising cleanup ([#1119](https://github.com/astral-sh/serc/pull/1119))
- Preserve live iterator ownership after named handler cleanup ([#1183](https://github.com/astral-sh/serc/pull/1183))
- Preserve local return borrowing through recovery finalizers ([#1045](https://github.com/astral-sh/serc/pull/1045))
- Preserve located exits after guarded finalizer arms ([#1227](https://github.com/astral-sh/serc/pull/1227))
- Preserve loop and manager exits around erased assertions ([#1205](https://github.com/astral-sh/serc/pull/1205))
- Preserve loop cleanup ownership after completed managers ([#1054](https://github.com/astral-sh/serc/pull/1054))
- Preserve loop exits after completed work and cleanup ([#1217](https://github.com/astral-sh/serc/pull/1217))
- Preserve loop-break entries into conditional cleanup ([#1137](https://github.com/astral-sh/serc/pull/1137))
- Preserve managed cleanup calls with conditional arguments ([#1143](https://github.com/astral-sh/serc/pull/1143))
- Preserve managed cleanup inside outer iterators ([#1142](https://github.com/astral-sh/serc/pull/1142))
- Preserve managed loop cleanup under protected finalizers ([#1096](https://github.com/astral-sh/serc/pull/1096))
- Preserve managed recovery paths into finalizers ([#1200](https://github.com/astral-sh/serc/pull/1200))
- Preserve manager cleanup entries past completed work ([#1157](https://github.com/astral-sh/serc/pull/1157))
- Preserve module cell ordering after assertion removal ([#1169](https://github.com/astral-sh/serc/pull/1169))
- Preserve monitoring marker owners after completed recovery ([#1232](https://github.com/astral-sh/serc/pull/1232))
- Preserve multiline pattern capture cleanup boundaries ([#1192](https://github.com/astral-sh/serc/pull/1192))
- Preserve named cleanup entry after guarded recovery ([#1077](https://github.com/astral-sh/serc/pull/1077))
- Preserve named-handler cleanup across iterator-free loops ([#1187](https://github.com/astral-sh/serc/pull/1187))
- Preserve nested branch exits in while-else loops ([#1064](https://github.com/astral-sh/serc/pull/1064))
- Preserve nested cleanup entries after guarded raises ([#1091](https://github.com/astral-sh/serc/pull/1091))
- Preserve nested recovery alternatives into cleanup ([#1100](https://github.com/astral-sh/serc/pull/1100))
- Preserve nested recovery exits before owning cleanup ([#1122](https://github.com/astral-sh/serc/pull/1122))
- Preserve normal cleanup after returning coroutine recovery ([#1125](https://github.com/astral-sh/serc/pull/1125))
- Preserve normal cleanup entry after guarded reraising recovery ([#1076](https://github.com/astral-sh/serc/pull/1076))
- Preserve normal exits after awaited else assertions ([#1099](https://github.com/astral-sh/serc/pull/1099))
- Preserve normal finalizer entries after raising recovery ([#1224](https://github.com/astral-sh/serc/pull/1224))
- Preserve null marker slots after setup compaction ([#1087](https://github.com/astral-sh/serc/pull/1087))
- Preserve original guard locations at finalizer exits ([#1219](https://github.com/astral-sh/serc/pull/1219))
- Preserve original scope footers through legacy exit copies ([#1233](https://github.com/astral-sh/serc/pull/1233))
- Preserve outer finalizer ownership during iterator cleanup ([#1061](https://github.com/astral-sh/serc/pull/1061))
- Preserve ownership through conditional manager cleanup ([#1053](https://github.com/astral-sh/serc/pull/1053))
- Preserve populated cleanup entries after guarded recovery ([#1185](https://github.com/astral-sh/serc/pull/1185))
- Preserve populated cleanup paths in nested loops ([#1095](https://github.com/astral-sh/serc/pull/1095))
- Preserve populated coroutine else-store entries ([#1166](https://github.com/astral-sh/serc/pull/1166))
- Preserve populated coroutine success exits into cleanup ([#1156](https://github.com/astral-sh/serc/pull/1156))
- Preserve populated nested recovery entries ([#1210](https://github.com/astral-sh/serc/pull/1210))
- Preserve recovery entries after completed work ([#1221](https://github.com/astral-sh/serc/pull/1221))
- Preserve recovery exits after attribute comprehensions ([#1134](https://github.com/astral-sh/serc/pull/1134))
- Preserve recovery finalizer borrowing after elif branches ([#1057](https://github.com/astral-sh/serc/pull/1057))
- Preserve return boundaries through branching cleanup calls ([#1090](https://github.com/astral-sh/serc/pull/1090))
- Preserve return cleanup order across nested scopes ([#1190](https://github.com/astral-sh/serc/pull/1190))
- Preserve return locations after asynchronous cleanup ([#1240](https://github.com/astral-sh/serc/pull/1240))
- Preserve return locations after managed branches ([#1238](https://github.com/astral-sh/serc/pull/1238))
- Preserve runtime finalizer loop entry and exit boundaries ([#1068](https://github.com/astral-sh/serc/pull/1068))
- Preserve runtime loop breaks into iterator cleanup ([#1144](https://github.com/astral-sh/serc/pull/1144))
- Preserve runtime loop recovery entries into finalizers ([#1214](https://github.com/astral-sh/serc/pull/1214))
- Preserve runtime nonterminal break paths into normal cleanup ([#1078](https://github.com/astral-sh/serc/pull/1078))
- Preserve runtime while false-edge borrowing into cleanup ([#1069](https://github.com/astral-sh/serc/pull/1069))
- Preserve runtime-loop cleanup beside nested recovery ([#1138](https://github.com/astral-sh/serc/pull/1138))
- Preserve saved iterator owners across erased branches ([#1188](https://github.com/astral-sh/serc/pull/1188))
- Preserve shared exits after completed generator conditions ([#1042](https://github.com/astral-sh/serc/pull/1042))
- Preserve shared exits in erased coroutine elif arms ([#1040](https://github.com/astral-sh/serc/pull/1040))
- Preserve shared exits in erased coroutine else branches ([#1041](https://github.com/astral-sh/serc/pull/1041))
- Preserve shared loop backedges in coroutines ([#1031](https://github.com/astral-sh/serc/pull/1031))
- Preserve shared loop backedges inside context managers ([#1038](https://github.com/astral-sh/serc/pull/1038))
- Preserve shared returns after manager-contained comprehensions ([#1179](https://github.com/astral-sh/serc/pull/1179))
- Preserve shared subscript returns across cleanup exits ([#1033](https://github.com/astral-sh/serc/pull/1033))
- Preserve short exits through erased finalizers ([#1118](https://github.com/astral-sh/serc/pull/1118))
- Preserve static cleanup for unpacked pattern alternatives ([#1172](https://github.com/astral-sh/serc/pull/1172))
- Preserve stored-call entries into normal cleanup ([#1145](https://github.com/astral-sh/serc/pull/1145))
- Preserve structured finalizer entries after loop breaks ([#1223](https://github.com/astral-sh/serc/pull/1223))
- Preserve surviving breaks into guarded cleanup ([#1140](https://github.com/astral-sh/serc/pull/1140))
- Preserve suspended cleanup entries and manager exits ([#1168](https://github.com/astral-sh/serc/pull/1168))
- Preserve terminal conditional name-store joins ([#1201](https://github.com/astral-sh/serc/pull/1201))
- Preserve terminal recovery through synchronous cleanup ([#1075](https://github.com/astral-sh/serc/pull/1075))
- Preserve tuple return copy order after conditional finalizers ([#1195](https://github.com/astral-sh/serc/pull/1195))
- Preserve tuple returns after guarded handler cleanup ([#1039](https://github.com/astral-sh/serc/pull/1039))
- Preserve tuple-unpack ownership before cleanup ([#1115](https://github.com/astral-sh/serc/pull/1115))
- Recognize unlabeled entry blocks in normal cleanup borrowing ([#1081](https://github.com/astral-sh/serc/pull/1081))
- Resolve assignment return locations after erased assertions ([#1228](https://github.com/astral-sh/serc/pull/1228))
- Retain authored break lines after normal try-exit removal ([#1079](https://github.com/astral-sh/serc/pull/1079))
- Retain cleanup ownership for handler returns through managers ([#1225](https://github.com/astral-sh/serc/pull/1225))
- Retire enclosing try before managed iterator cleanup ([#1175](https://github.com/astral-sh/serc/pull/1175))
- Retire inner try ownership before nested handler iterator cleanup ([#1180](https://github.com/astral-sh/serc/pull/1180))
- Retire inner try regions before return iterator cleanup ([#1060](https://github.com/astral-sh/serc/pull/1060))
- Share annotation-only subscript traversal ([#1246](https://github.com/astral-sh/serc/pull/1246))
- Share original marker-prefix lookup ([#1248](https://github.com/astral-sh/serc/pull/1248))
- Share suspension detection across cleanup checks ([#1247](https://github.com/astral-sh/serc/pull/1247))
- Test reference identity validation without temporary executables ([#1189](https://github.com/astral-sh/serc/pull/1189))
- Trim historical compatibility documentation ([#1244](https://github.com/astral-sh/serc/pull/1244))

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
