// The topology manager's node-kind vocabulary, rendered through the catalog.
//
// A page bundle runs DOM initialization at import, so a label only a bundle can
// produce cannot be asserted by a test. This module holds the words instead: the
// kind comes from the node-id prefix the topology source emits, and the word
// comes from the active catalog, so a hardcoded English switch cannot creep back
// in without failing `topology_labels.test.mjs`.

import {t} from "./i18n/index.js";

// The synthetic id this UI uses for the tree's root; never a server id.
// translatable: false — "__topology_root__" is the synthetic root node id, matched literally
export const ROOT_NODE_ID = "__topology_root__";

// translatable: false — "uisp:site:" is a node-id prefix emitted by the UISP topology source
const UISP_SITE_PREFIX = "uisp:site:";
// translatable: false — "uisp:device:" is a node-id prefix emitted by the UISP topology source
const UISP_DEVICE_PREFIX = "uisp:device:";
// translatable: false — "libreqos:generated:" is a node-id prefix emitted for generated nodes
const GENERATED_PREFIX = "libreqos:generated:";

/**
 * The operator-facing kind of a topology node.
 * @param {string|null|undefined} nodeId Topology node id, or null when nothing is selected.
 * @returns {string} The localized kind label.
 */
export function nodeKindLabel(nodeId) {
    if (!nodeId) return t("topology.node_kind.node");
    if (nodeId === ROOT_NODE_ID) return t("topology.node_kind.root");
    if (nodeId.startsWith(UISP_SITE_PREFIX)) return t("topology.node_kind.site");
    if (nodeId.startsWith(UISP_DEVICE_PREFIX)) return t("topology.node_kind.device");
    if (nodeId.startsWith(GENERATED_PREFIX)) return t("topology.node_kind.generated");
    return t("topology.node_kind.node");
}

/**
 * Ranking weight of a node kind, used when ordering search matches: a site
 * outranks a generated node, which outranks a device, and anything else is 0.
 * @param {string|null|undefined} nodeId Topology node id.
 * @returns {number}
 */
export function nodeKindPriority(nodeId) {
    if (!nodeId) return 0;
    if (nodeId.startsWith(UISP_SITE_PREFIX)) return 3;
    if (nodeId.startsWith(GENERATED_PREFIX)) return 2;
    if (nodeId.startsWith(UISP_DEVICE_PREFIX)) return 1;
    return 0;
}
