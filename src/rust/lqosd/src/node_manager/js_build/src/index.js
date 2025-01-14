import {Dashboard} from "./dashlets/dashboard";
import {checkForUpgrades} from "./toasts/version_check";
import {sponsorTag} from "./toasts/sponsor_us";
import {globalWarningToasts} from "./toasts/global_warnings";
import {showTimeControls} from "./components/timescale";

showTimeControls("timescale");
checkForUpgrades();
sponsorTag("toasts");
globalWarningToasts();
const dashboard = new Dashboard("dashboard");
dashboard.build();
