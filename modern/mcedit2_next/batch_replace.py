"""One-pass, original-format multi-rule block replacement.

Read every selection cell exactly once. A cell matches its *original* state,
not a block produced by an earlier rule in the same operation. Unknown modded
BlockState identities are never converted through a registry or version map.
Protected Block Entity positions may be skipped, but never overwritten.
"""
from __future__ import annotations

from dataclasses import dataclass

MAX_BATCH_RULES = 32


@dataclass(frozen=True)
class BatchReplacePreview:
    total_blocks: int
    matches: int
    changes: int
    protected_be: int
    skipped_be: int
    rules: int


def parse_batch_rules(text: str, make_block):
    """Parse a bounded, user-authored 'source -> destination' rule list."""
    if type(text) is not str or not callable(make_block):
        raise ValueError("Expected native replacement rules and a BlockState parser")
    entries = []
    for lineno, raw in enumerate(text.splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if line.count("->") != 1:
            raise ValueError(f"Rule on line {lineno} must contain one '->'")
        source, target = (part.strip() for part in line.split("->"))
        if not source or not target:
            raise ValueError(f"Rule on line {lineno} has an empty source or target")
        entries.append((make_block(source), make_block(target)))
        if len(entries) > MAX_BATCH_RULES:
            raise ValueError(f"No more than {MAX_BATCH_RULES} batch rules")
    if not entries:
        raise ValueError("Enter at least one replacement rule")
    return tuple(entries)


def _key(state, exact_state):
    if exact_state:
        return str(state)
    return state.name if hasattr(state, "name") else str(state).split("[", 1)[0]


def plan_batch_replace(world, box, rules, *, exact_state=False,
                       skip_block_entities=True):
    """Produce only a read-only preview and a frozen, unique write plan.

    Must be called behind the session's volume guard. Native edits are not
    staged until EditSession._commit preflights the entire write set.
    """
    if type(exact_state) is not bool or type(skip_block_entities) is not bool:
        raise ValueError("Replacement options must be booleans")
    if not isinstance(rules, (list, tuple)) or not 1 <= len(rules) <= MAX_BATCH_RULES:
        raise ValueError(f"Specify 1–{MAX_BATCH_RULES} native replacement rules")
    checker = getattr(world, "has_block_entity", None)
    if not callable(checker):
        raise ValueError("Native Block Entity presence checks are required")
    rule_map = {}
    for entry in rules:
        if not isinstance(entry, (list, tuple)) or len(entry) != 2:
            raise ValueError("Each replacement rule must be a (source, target) pair")
        source, target = entry
        key = _key(source, exact_state)
        if key in rule_map:
            raise ValueError(f"Duplicate matching source: {key}")
        rule_map[key] = target
    matches = protected = skipped = 0
    updates = []
    for point in box.points():
        before = world.get_block(*point)
        key = _key(before, exact_state)
        if key not in rule_map:
            continue
        matches += 1
        after = rule_map[key]
        if before == after:
            continue
        if checker(*point):
            protected += 1
            if skip_block_entities:
                skipped += 1
                continue
        updates.append((point, after))
    preview = BatchReplacePreview(box.volume, matches, len(updates),
                                  protected, skipped, len(rules))
    return preview, updates
