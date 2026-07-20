import {disposeTooltipsWithin} from "../lq_js_common/helpers/tooltips.js";

export function clearElement(target, targetLength = 0) {
    disposeTooltipsWithin(target);
    while (target.children.length > targetLength) {
        target.removeChild(target.lastChild);
    }
}
