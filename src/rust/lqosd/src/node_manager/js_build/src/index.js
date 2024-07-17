import {Dashboard} from "./dashlets/dashboard";
import {checkForUpgrades} from "./toasts/version_check";
import {initRedact} from "./helpers/redact";
import {sponsorTag} from "./toasts/sponsor_us";

checkForUpgrades("toasts");
sponsorTag("toasts")
const dashboard = new Dashboard("dashboard");
dashboard.build();
