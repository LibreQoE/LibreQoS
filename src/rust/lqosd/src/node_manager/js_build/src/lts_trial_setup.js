import {t} from "./i18n/index.js";
import {buildInsightUrl, sendWsRequest} from "./lts_trial_shared";

const setupStatusClasses = [
    'alert-primary',
    'alert-success',
    'alert-warning',
    'alert-danger',
    'alert-secondary',
];
const defaultInsightBaseUrl = 'https://insight.libreqos.com/';
let insightBaseUrl = defaultInsightBaseUrl;

function setSetupStatus(message, tone = 'primary', spinning = true) {
    const status = $('#signupSetupStatus');
    const statusText = $('#signupSetupStatusText');
    const spinner = status.find('.spinner-border');

    if (!status.length || !statusText.length) {
        return;
    }

    status.removeClass(setupStatusClasses.join(' ')).addClass(`alert-${tone}`);
    statusText.text(message);

    if (spinning) {
        spinner.show();
    } else {
        spinner.hide();
    }
}

function setSetupHelp(message) {
    $('#signupSetupHelp').text(message);
}

function getErrorMessage(error, fallback) {
    if (error && typeof error.message === 'string' && error.message.trim().length > 0) {
        return error.message.trim();
    }
    return fallback;
}

async function fetchTrialConfig() {
    try {
        const response = await sendWsRequest("LtsTrialConfigResult", { LtsTrialConfig: {} });
        const data = response && response.data ? response.data : {};
        if (typeof data.lts_url === 'string' && data.lts_url.trim().length > 0) {
            insightBaseUrl = data.lts_url;
        }
    } catch (error) {
        // translatable: false — "Unable to load LTS trial config, using default Insight URL." is a console diagnostic, never rendered
        console.warn('Unable to load LTS trial config, using default Insight URL.', error);
        insightBaseUrl = defaultInsightBaseUrl;
    }
}

function redirectToInsightSignup(claimId) {
    window.location.href = buildInsightUrl(
        insightBaseUrl,
        `su/signup1.html?${encodeURIComponent(claimId)}`,
    );
}

async function startSignupSession() {
    setSetupStatus(t("trial.setup.creating"));
    setSetupHelp(t("trial.setup.creating_help"));
    await fetchTrialConfig();

    try {
        const response = await sendWsRequest("LtsStartSignupResult", { LtsStartSignup: {} });
        const claimId = response && typeof response.claim_id === 'string'
            ? response.claim_id.trim()
            : '';

        if (!claimId) {
            throw new Error(t("trial.setup.missing_claim"));
        }

        setSetupStatus(t("trial.setup.ready"), 'success', false);
        setSetupHelp(t("trial.setup.ready_help"));
        redirectToInsightSignup(claimId);
    } catch (error) {
        // translatable: false — "Failed to start Insight signup session." is a console diagnostic, never rendered
        console.error('Failed to start Insight signup session.', error);
        setSetupStatus(
            getErrorMessage(error, t("trial.setup.start_failed")),
            'danger',
            false,
        );
        setSetupHelp(t("trial.setup.retry_help"));
    }
}

$(document).ready(() => {
    setSetupStatus(t("trial.setup.preparing"));
    void startSignupSession();
});
