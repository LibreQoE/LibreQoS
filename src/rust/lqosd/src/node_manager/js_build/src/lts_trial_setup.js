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
    setSetupStatus('Creating your Insight signup session…');
    setSetupHelp('LibreQoS is contacting Insight and preparing your claim.');
    await fetchTrialConfig();

    try {
        const response = await sendWsRequest("LtsStartSignupResult", { LtsStartSignup: {} });
        const claimId = response && typeof response.claim_id === 'string'
            ? response.claim_id.trim()
            : '';

        if (!claimId) {
            throw new Error('Missing claim ID from signup response.');
        }

        setSetupStatus('Signup session ready. Redirecting to Insight…', 'success', false);
        setSetupHelp('Your signup session is ready. Continuing to Insight now.');
        redirectToInsightSignup(claimId);
    } catch (error) {
        console.error('Failed to start Insight signup session.', error);
        setSetupStatus(
            getErrorMessage(error, 'Unable to start the Insight signup session right now.'),
            'danger',
            false,
        );
        setSetupHelp(
            'Retry this page, use the direct signup form, or validate an existing license key below.',
        );
    }
}

$(document).ready(() => {
    setSetupStatus('Preparing the Insight signup handoff…');
    void startSignupSession();
});
