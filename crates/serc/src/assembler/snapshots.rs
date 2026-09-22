//! Opt-in, thread-local traces of the real pipeline; normal tests record nothing.

use std::cell::RefCell;
use std::fmt::Write as _;

use super::{AssemblerStage, ExceptionRegion, Item};

thread_local! {
    static TRACE: RefCell<Option<String>> = const { RefCell::new(None) };
}

pub(super) fn record(stage: AssemblerStage, items: &[Item], regions: &[ExceptionRegion]) {
    if !matches!(
        stage,
        AssemblerStage::RemoveUnreachableInitial
            | AssemblerStage::DuplicateExitBlocks
            | AssemblerStage::OptimizeLoadFast
            | AssemblerStage::FinalLayout
    ) {
        return;
    }
    TRACE.with_borrow_mut(|trace| {
        let Some(trace) = trace else { return };
        writeln!(trace, "\n== {} ==", stage.name()).unwrap();
        for item in items {
            match item {
                Item::Label(label) => writeln!(trace, "L{}:", label.index()).unwrap(),
                Item::Instruction(instruction) => {
                    let location = instruction.location;
                    writeln!(
                        trace,
                        "  {} {:?} @ {}:{}-{}:{} depth={:?} exception={:?}",
                        instruction.opcode.name(),
                        instruction.operand(),
                        location.line,
                        location.column,
                        location.end_line,
                        location.end_column,
                        instruction.depth_after(),
                        instruction.exception_policy()
                    )
                    .unwrap();
                }
            }
        }
        for region in regions {
            writeln!(
                trace,
                "  handler L{}..L{} -> L{} depth={} lasti={} fallback={} priority={:?}",
                region.start.index(),
                region.end.index(),
                region.target.index(),
                region.depth,
                region.preserve_lasti,
                region.is_scope_fallback,
                region.priority
            )
            .unwrap();
        }
    });
}

struct Capture;

impl Capture {
    fn start() -> Self {
        TRACE.with_borrow_mut(|trace| {
            assert!(trace.is_none(), "nested assembler trace");
            *trace = Some(String::new());
        });
        Self
    }

    fn finish(self) -> String {
        let trace = TRACE.with_borrow_mut(|trace| trace.take().unwrap());
        drop(self);
        trace
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        TRACE.with_borrow_mut(|trace| *trace = None);
    }
}

#[test]
fn branch_and_exception_pipeline() {
    let capture = Capture::start();
    crate::compile("def choose(flag, value):\n    try:\n        if flag:\n            return value\n        return 0\n    except ValueError:\n        return -1\n", "pipeline.py").unwrap();
    insta::assert_snapshot!(capture.finish());
}
