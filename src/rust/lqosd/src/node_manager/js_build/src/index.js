import {Dashboard} from "./dashlets/dashboard";
import {checkForUpgrades} from "./toasts/version_check";
import {initRedact} from "./helpers/redact";

initRedact();
checkForUpgrades("toasts");
const dashboard = new Dashboard("dashboard");
dashboard.build();
