import {Dashboard} from "./dashlets/dashboard";
import {checkForUpgrades} from "./toasts/version_check";

checkForUpgrades("toasts");
const dashboard = new Dashboard("dashboard");
dashboard.build();
