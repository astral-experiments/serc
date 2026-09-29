## synchronous_manager_cleanup_keeps_one_byte_backward_jump

Original manager cleanup jumps have labeled successors. Their source label
budget keeps the 3.12 cleanup backedge at 255 rather than 256.

## terminal_nested_branches_keep_native_jump_labels

The terminal nested branch needs its original dead join and default return until
3.12 captures jump operands, even though both disappear.
