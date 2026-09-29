def outer(assignment, meta, block_scope_mergeability):
    def render_block_scope_handle(handle: Handle, handle_has_approval: bool) -> str:
        res = link_to_handle_with_assignment(handle, assignment, meta)
        match handle:
            case UserHandle(_) as user:
                if user not in block_scope_mergeability.approving_users and handle_has_approval:
                    res = f"~{res}~"
            case TeamHandle(_, _) as team:
                if users := block_scope_mergeability.approving_teams.get(team, set()):
                    via = " + ".join(link_to_handle(user) for user in _sorted_handles(users))
                    res = f"{res} _via_ {via}"
                elif handle_has_approval:
                    res = f"~{res}~"
        return res
    return render_block_scope_handle
