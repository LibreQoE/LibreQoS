"""
Internal planner for stable, damped item-to-bin assignments.

This module exposes a minimal interface initially, to be expanded in later
steps. The baseline implementation uses a simple greedy balancing strategy
as a placeholder to replace the external binpacking dependency.
"""

from typing import Dict, List, Tuple, Any, Iterable
import hashlib
import math
import json
import os
import time


def plan_assignments(
    items: List[Dict[str, Any]],
    bins: List[Dict[str, Any]],
    capacities: Dict[str, float],
    previous_assignments: Dict[str, str],
    now_ts: float,
    params: Dict[str, Any],
) -> Tuple[Dict[str, str], List[str]]:
    """
    Compute item -> bin assignments with a repair-oriented approach.

    items: list of { "id": str, "weight": float }
    bins: list of { "id": str }
    capacities: { bin_id: capacity_float }
    previous_assignments: { item_id: bin_id }
    now_ts: current UNIX timestamp
    params: planner parameters (optional keys):
        - candidate_set_size: int
        - headroom: float in [0,1)
        - alpha: float >= 0
        - hysteresis_threshold: float >= 0
        - cooldown_seconds: int >= 0
        - move_budget_per_run: int >= 0
        - salt: str
        - last_change_ts_by_item: { item_id: unix_ts }
        - movement_penalty_k: float >= 0 (default 0)
        - penalty_tau_seconds: float > 0 (default 3600)

    Returns (assignments, changed_items)
    """
    # Defaults
    L = int(params.get("candidate_set_size", 4))
    headroom = float(params.get("headroom", 0.05))
    alpha = float(params.get("alpha", 0.1))
    hysteresis = float(params.get("hysteresis_threshold", 0.03))
    cooldown_s = int(params.get("cooldown_seconds", 3600))
    salt = str(params.get("salt", "default_salt"))
    last_change_ts = params.get("last_change_ts_by_item", {}) or {}
    penalty_k = float(params.get("movement_penalty_k", 0.0))
    penalty_tau = float(params.get("penalty_tau_seconds", 3600.0))
    freeze_moves = bool(params.get("freeze_moves", False))

    bin_ids: List[str] = [b["id"] for b in bins]
    cap = {bid: float(capacities.get(bid, 1.0)) for bid in bin_ids}
    # Avoid zero/negative capacity
    for bid, c in list(cap.items()):
        if c <= 0:
            cap[bid] = 1e-9

    # Normalize items
    item_list: List[Tuple[str, float]] = []
    for it in items:
        it_id = str(it.get("id"))
        w = float(it.get("weight", 1.0))
        item_list.append((it_id, w))

    # Current assignment: keep valid previous assignments
    assignment: Dict[str, str] = {}
    load: Dict[str, float] = {bid: 0.0 for bid in bin_ids}
    for it_id, w in item_list:
        prev = previous_assignments.get(it_id)
        if prev in cap:
            assignment[it_id] = prev
            load[prev] += w

    # Cold start: assign items without previous placement via greedy using HRW candidates
    missing = [(it_id, w) for it_id, w in item_list if it_id not in assignment]
    weights_by_bin = {bid: cap[bid] for bid in bin_ids}
    missing.sort(key=lambda t: (-t[1], t[0]))
    initially_assigned: List[str] = []
    for it_id, w in missing:
        candidates = hrw_candidates(it_id, bin_ids, weights_by_bin, max(L, 1), salt)
        # Choose the least loaded among candidates; break ties by HRW rank, not lexicographic bin id
        if candidates:
            rank = {bid: idx for idx, bid in enumerate(candidates)}
            tgt = min(candidates, key=lambda bid: (load[bid] / cap[bid], rank[bid]))
        else:
            # As a fallback, rank all bins by HRW order to avoid lexicographic bias
            all_rank_list = hrw_candidates(it_id, bin_ids, weights_by_bin, len(bin_ids), salt)
            all_rank = {bid: idx for idx, bid in enumerate(all_rank_list)}
            tgt = min(bin_ids, key=lambda bid: (load[bid] / cap[bid], all_rank.get(bid, 1_000_000)))
        assignment[it_id] = tgt
        load[tgt] += w
        initially_assigned.append(it_id)

    def score(loads: Dict[str, float]) -> float:
        s = 0.0
        for bid in bin_ids:
            denom = cap[bid] * max(1.0 - headroom, 1e-6)
            lj = loads[bid] / denom
            over = max(0.0, lj - 1.0)
            s += over * over
        # Add a small balancing term
        if alpha > 0.0:
            for bid in bin_ids:
                lj = loads[bid] / cap[bid]
                s += alpha * (lj * lj)
        return s

    # Initial score
    base_score = score(load)

    # Decide moves by considering HRW top-L candidates per item
    move_budget = params.get("move_budget_per_run")
    if move_budget is None:
        move_budget = max(1, min(32, int(0.01 * max(1, len(item_list)))))
    moves_left = int(move_budget)

    # Worklist: heavier items first
    work = sorted(item_list, key=lambda t: (-t[1], t[0]))
    changed_items: List[str] = []

    for it_id, w in work:
        if moves_left <= 0:
            break
        cur = assignment.get(it_id)
        candidates = hrw_candidates(it_id, bin_ids, weights_by_bin, max(L, 1), salt)
        # Ensure current bin is considered
        if cur and cur not in candidates:
            candidates.append(cur)
        # Hard stickiness option: only move off an overloaded bin
        if freeze_moves:
            if not cur:
                # Nothing to freeze; should not happen here because cold-start assigns above
                continue
            denom_cur = cap[cur] * max(1.0 - headroom, 1e-6)
            if (load[cur] / denom_cur) <= 1.0:
                # Current bin is not overloaded relative to headroom; skip any move
                continue
        best_gain = 0.0
        best_tgt = cur
        for tgt in candidates:
            if tgt == cur:
                continue
            # simulate move
            if cur:
                load[cur] -= w
            load[tgt] += w
            new_score = score(load)
            # revert
            load[tgt] -= w
            if cur:
                load[cur] += w
            gain = base_score - new_score
            if gain > best_gain:
                best_gain = gain
                best_tgt = tgt

        if best_tgt and best_tgt != cur:
            # Cooldown check
            last_ts = float(last_change_ts.get(it_id, 0.0))
            age = max(0.0, float(now_ts) - last_ts)
            if age < float(cooldown_s):
                continue
            # Movement penalty
            penalty = 0.0
            if penalty_k > 0.0 and penalty_tau > 0.0:
                penalty = penalty_k * float(w) * math.exp(-age / penalty_tau)
            net_gain = best_gain - penalty
            # Hysteresis threshold: relative to base_score magnitude or absolute small value if base is tiny
            threshold = max(hysteresis * max(1.0, base_score), 1e-6)
            if net_gain >= threshold:
                # apply move
                if cur:
                    load[cur] -= w
                load[best_tgt] += w
                assignment[it_id] = best_tgt
                changed_items.append(it_id)
                # update base score and budget
                base_score -= best_gain
                moves_left -= 1

    # Mark initial placements as changes so callers can timestamp them
    for iid in initially_assigned:
        if iid not in changed_items:
            changed_items.append(iid)

    return assignment, changed_items


def load_state(path: str) -> Dict[str, Any]:
    """
    Load planner state from JSON. If missing or invalid, return a fresh state.
    """
    fresh = {
        "algo_version": "v1",
        "salt": None,
        "assignments": {},
        "last_change_ts": {},
        "updated_at": None,
    }
    try:
        if not os.path.exists(path):
            return fresh
        with open(path, 'r', encoding='utf-8') as f:
            data = json.load(f)
        # Minimal validation
        if not isinstance(data, dict):
            return fresh
        for k in ["algo_version", "salt", "assignments", "last_change_ts", "updated_at"]:
            if k not in data:
                data[k] = fresh.get(k)
        if data.get("salt") is None:
            data["salt"] = _generate_salt()
        # TTL: invalidate if older than 24 hours
        try:
            updated_at = float(data.get("updated_at") or 0.0)
            if updated_at <= 0.0:
                return data
            now = time.time()
            if (now - updated_at) > 24 * 3600:
                # Regenerate: return a fresh state with a new salt
                fresh["salt"] = _generate_salt()
                return fresh
        except Exception:
            # If parsing fails, return fresh
            fresh["salt"] = _generate_salt()
            return fresh
        return data
    except Exception:
        return fresh


def save_state(path: str, state: Dict[str, Any]) -> None:
    """
    Save planner state to JSON. Creates parent directory if necessary.
    """
    try:
        os.makedirs(os.path.dirname(path), exist_ok=True)
    except Exception:
        pass
    state = dict(state)
    state["updated_at"] = time.time()
    if state.get("salt") is None:
        state["salt"] = _generate_salt()
    with open(path, 'w', encoding='utf-8') as f:
        json.dump(state, f, indent=2, sort_keys=True)


def _generate_salt() -> str:
    return hashlib.sha256(str(time.time_ns()).encode('utf-8')).hexdigest()


def hrw_candidates(
    item_id: str,
    bin_ids: List[str],
    weights_by_bin: Dict[str, float],
    L: int,
    salt: str,
) -> List[str]:
    """
    Weighted Rendezvous (HRW) hashing: return top-L candidate bins for an item.

    score(bin) = weight(bin) / -ln(U), where U derives deterministically from
    SHA-256(salt || item_id || bin_id).
    """
    scores: List[Tuple[float, str]] = []
    for bid in bin_ids:
        w = float(weights_by_bin.get(bid, 1.0))
        # Deterministic uniform in (0,1)
        h = hashlib.sha256((salt + '|' + str(item_id) + '|' + str(bid)).encode('utf-8')).digest()
        # Use first 8 bytes for a 64-bit integer
        r = int.from_bytes(h[:8], byteorder='big', signed=False)
        # Map to (0,1). Avoid 0 by adding a tiny epsilon.
        U = max((r / 2**64), 1e-12)
        score = w / (-math.log(U)) if U < 1.0 else float('inf')
        scores.append((score, bid))
    scores.sort(key=lambda t: (-t[0], t[1]))
    if L <= 0:
        return []
    return [bid for _, bid in scores[:min(L, len(scores))]]
