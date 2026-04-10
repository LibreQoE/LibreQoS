def is_virtual_node(node_dict):
    """
    Returns True if a network.json node is marked as virtual (logical-only).

    Supported markers:
      - {"virtual": true} (preferred)
      - {"type": "virtual"} (legacy compatibility)
    """
    try:
        if not isinstance(node_dict, dict):
            return False
        if bool(node_dict.get("virtual", False)):
            return True
        t = node_dict.get("type", "")
        return isinstance(t, str) and t.lower() == "virtual"
    except Exception:
        return False


def build_logical_to_physical_node_map(logical_network):
    """
    Returns (mapping, virtual_nodes) where mapping is a dict of:
      logical_node_name -> nearest_non_virtual_ancestor_name (or None if none exists).

    Non-virtual nodes map to themselves, so callers can safely look up any node name.
    """
    mapping = {}
    virtual_nodes = []

    def recurse(level, nearest_real_ancestor):
        if not isinstance(level, dict):
            return
        for name, node in level.items():
            if not isinstance(name, str) or not isinstance(node, dict):
                continue

            if is_virtual_node(node):
                mapping[name] = nearest_real_ancestor
                virtual_nodes.append(name)
                children = node.get("children", None)
                if isinstance(children, dict):
                    recurse(children, nearest_real_ancestor)
            else:
                mapping[name] = name
                children = node.get("children", None)
                if isinstance(children, dict):
                    recurse(children, name)

    recurse(logical_network, None)
    return mapping, virtual_nodes


def build_physical_network(logical_network):
    """
    Builds a physical HTB topology by removing virtual nodes and promoting their children
    into the virtual node's parent level.

    Raises ValueError on name collisions caused by promotion.
    """
    if not isinstance(logical_network, dict):
        return {}

    physical = {}
    for name, node in logical_network.items():
        if not isinstance(name, str) or not isinstance(node, dict):
            continue

        if is_virtual_node(node):
            children = node.get("children", None)
            if isinstance(children, dict):
                promoted = build_physical_network(children)
                for child_name, child_node in promoted.items():
                    if child_name in physical:
                        raise ValueError(
                            f"Virtual node promotion collision: '{child_name}' already exists at this level."
                        )
                    physical[child_name] = child_node
            continue

        new_node = dict(node)
        if "children" in new_node and isinstance(new_node.get("children"), dict):
            new_children = build_physical_network(new_node["children"])
            if new_children:
                new_node["children"] = new_children
            else:
                new_node.pop("children", None)

        # Keep physical topology clean; virtual markers are logical-only.
        new_node.pop("virtual", None)
        physical[name] = new_node

    return physical


def collect_physical_parent_node_aliases(network):
    """
    Returns a mapping of acceptable parent-node aliases to the physical node key
    that exists in the shaping tree.

    This lets shaping-time code resolve attachment-style names that may survive
    in `ShapedDevices.csv` even after the physical tree is flattened or promoted.
    """
    aliases = {}

    def recurse(level):
        if not isinstance(level, dict):
            return

        for key, node in level.items():
            if not isinstance(key, str) or not isinstance(node, dict):
                continue

            candidates = [key, node.get("name"), node.get("active_attachment_name")]
            for candidate in candidates:
                if candidate is None:
                    continue
                alias = str(candidate).strip()
                if not alias:
                    continue
                aliases.setdefault(alias, key)

            children = node.get("children")
            if isinstance(children, dict):
                recurse(children)

    recurse(network)
    return aliases


def collect_physical_parent_node_ids(network):
    """
    Returns a mapping of `network.json` node IDs to the physical node key that
    exists in the shaping tree.

    This lets shaping-time code resolve circuits by stable node identifier
    instead of relying only on display names.
    """
    node_ids = {}

    def recurse(level):
        if not isinstance(level, dict):
            return

        for key, node in level.items():
            if not isinstance(key, str) or not isinstance(node, dict):
                continue

            node_id = node.get("id")
            if node_id is not None:
                node_id = str(node_id).strip()
                if node_id:
                    node_ids.setdefault(node_id, key)

            children = node.get("children")
            if isinstance(children, dict):
                recurse(children)

    recurse(network)
    return node_ids
