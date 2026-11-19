import {ShaperWorldMapDashlet} from "./world_map_down";
import {t} from "../i18n/index.js";

export class ShaperWorldMapUp extends ShaperWorldMapDashlet {
    constructor(slot){ super(slot, t("dashlet.name.shaper_world_map_upload")); }
}
