import {Dashboard} from "./dashlets/dashboard";
import {checkForUpgrades} from "./toasts/version_check";
import {sponsorTag} from "./toasts/sponsor_us";
import {globalWarningToasts} from "./toasts/global_warnings";

checkForUpgrades("toasts");
sponsorTag("toasts");
globalWarningToasts();
const dashboard = new Dashboard("dashboard");
dashboard.build();
