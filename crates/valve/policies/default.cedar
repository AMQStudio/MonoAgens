// Default compiled-in MonoAgens Valve Cedar policy bundle.
// Absolute Line 1: only Actor::"owner" may trigger SelfModify or identity_changing capabilities.

// Inbound percepts require a non-empty declared audience and an attested or known actor.
permit(
    principal,
    action == Action::"InboundPercept",
    resource
) when {
    context.has_audience &&
    (principal == Actor::"owner" ||
     principal in Role::"known_actor" ||
     principal in Role::"room_present" ||
     principal in Role::"physically_present")
};

// Context assembly requires an active VisibleScope.
permit(
    principal,
    action == Action::"ContextAssemble",
    resource
) when {
    context.has_scope
};

// Owner may invoke any capability including SelfModify and identity_changing acts.
permit(
    principal == Actor::"owner",
    action == Action::"SelfModify",
    resource
);

permit(
    principal == Actor::"owner",
    action == Action::"InvokeCapability",
    resource
);

// Non-owner actors with attested roles or known identity may invoke non-identity-changing capabilities.
permit(
    principal,
    action == Action::"InvokeCapability",
    resource
) when {
    !resource.identity_changing &&
    (principal in Role::"known_actor" ||
     principal in Role::"room_present" ||
     principal in Role::"physically_present")
};

// Memory-write door: owner may write trusted items/procedures; non-owners cannot write tainted or procedure items.
permit(
    principal == Actor::"owner",
    action == Action::"WriteMemory",
    resource
) when {
    !context.tainted &&
    (!resource.is_procedure || resource.owner_signed)
};

permit(
    principal,
    action == Action::"WriteMemory",
    resource
) when {
    !context.tainted &&
    !resource.is_procedure &&
    resource.trusted
};

// Belt-and-suspenders explicit forbid for Absolute Line 1.
forbid(
    principal,
    action == Action::"SelfModify",
    resource
) when {
    principal != Actor::"owner"
};

forbid(
    principal,
    action == Action::"InvokeCapability",
    resource
) when {
    principal != Actor::"owner" &&
    resource.identity_changing
};
