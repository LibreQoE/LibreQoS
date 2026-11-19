import {escapeHtml} from "./i18n/escape.mjs";
import {formatNumber} from "./i18n/format.js";
import {t, tAttr, tn} from "./i18n/index.js";
import {PLACEHOLDER_TEASERS} from "./lts_teasers_shared";
import {buildSignupApiUrl, sendWsRequest, wsClient} from "./lts_trial_shared";
import {regionName} from "./region_names.mjs";

// Paddle-compatible countries (excluding embargoed nations). Only the ISO codes
// live here: `initializeCountrySelector` labels each option with the engine's
// CLDR region name for the active locale, so the list carries no copy.
const ALLOWED_COUNTRY_CODES = [
    'ad', 'ae', 'ag', 'ai', 'al', 'am', 'ao', 'ar', 'at', 'au', 'aw', 'ax', 'az', 'ba', 'bb',
    'bd', 'be', 'bf', 'bg', 'bh', 'bi', 'bj', 'bl', 'bm', 'bn', 'bo', 'bq', 'br', 'bs', 'bt',
    'bw', 'bz', 'ca', 'cc', 'cd', 'cf', 'cg', 'ch', 'ci', 'ck', 'cl', 'cm', 'cn', 'co', 'cr',
    'cv', 'cw', 'cx', 'cy', 'cz', 'de', 'dj', 'dk', 'dm', 'do', 'dz', 'ec', 'ee', 'eg', 'er',
    'es', 'et', 'fi', 'fj', 'fk', 'fm', 'fo', 'fr', 'ga', 'gb', 'gd', 'ge', 'gf', 'gg', 'gh',
    'gi', 'gl', 'gm', 'gn', 'gp', 'gq', 'gr', 'gs', 'gt', 'gu', 'gw', 'gy', 'hk', 'hm', 'hn',
    'hr', 'ht', 'hu', 'id', 'ie', 'il', 'im', 'in', 'io', 'iq', 'is', 'it', 'je', 'jm', 'jo',
    'jp', 'ke', 'kg', 'kh', 'ki', 'km', 'kn', 'kr', 'kw', 'ky', 'kz', 'la', 'lb', 'lc', 'li',
    'lk', 'lr', 'ls', 'lt', 'lu', 'lv', 'ly', 'ma', 'mc', 'md', 'me', 'mf', 'mg', 'mh', 'mk',
    'ml', 'mm', 'mn', 'mo', 'mp', 'mq', 'mr', 'ms', 'mt', 'mu', 'mv', 'mw', 'mx', 'my', 'mz',
    'na', 'nc', 'ne', 'nf', 'ng', 'ni', 'nl', 'no', 'np', 'nr', 'nu', 'nz', 'om', 'pa', 'pe',
    'pf', 'pg', 'ph', 'pk', 'pl', 'pm', 'pn', 'pr', 'ps', 'pt', 'pw', 'py', 'qa', 're', 'ro',
    'rs', 'rw', 'sa', 'sb', 'sc', 'se', 'sg', 'sh', 'si', 'sj', 'sk', 'sl', 'sm', 'sn', 'so',
    'sr', 'ss', 'st', 'sv', 'sx', 'sz', 'tc', 'td', 'tf', 'tg', 'th', 'tj', 'tk', 'tl', 'tm',
    'tn', 'to', 'tr', 'tt', 'tv', 'tw', 'tz', 'ua', 'ug', 'um', 'us', 'uy', 'uz', 'va', 'vc',
    've', 'vg', 'vi', 'vn', 'vu', 'wf', 'ws', 'ye', 'yt', 'za', 'zm', 'zw',
    // Note: Excluded embargoed countries: Cuba (cu), Iran (ir), North Korea (kp), Russia (ru), Syria (sy)
];


// State management
let currentTeasers = [];
let nodeId = null;
let ltsBaseUrl = 'https://insight.libreqos.com/';

// Initialize the page
$(document).ready(async function() {
    initializeCountrySelector();
    await fetchNodeId();  // Fetch config first to get the correct URL
    loadTeasers();        // Then load teasers with the correct URL
    fetchCircuitCount();
    attachEventHandlers();
    openInitialViewFromQuery();
    
    // Apply dark mode if needed
    if (isDarkMode()) {
        applyDarkMode();
    }
    
    // Handle window resize for carousel
    let resizeTimer;
    $(window).on('resize', function() {
        clearTimeout(resizeTimer);
        resizeTimer = setTimeout(function() {
            if (currentTeasers.length > 0) {
                displayTeasers();
            }
        }, 250);
    });
});

// Dark mode detection
function isDarkMode() {
    const currentTheme = localStorage.getItem('theme');
    if (currentTheme === null) {
        // translatable: false — "(prefers-color-scheme: dark)" is the media feature the browser matches
        return window.matchMedia('(prefers-color-scheme: dark)').matches;
    }
    return currentTheme === 'dark';
}

// Apply dark mode styles
function applyDarkMode() {
    // Bootstrap 5 dark mode is handled by data-bs-theme attribute
    // This should be set by the parent template, but we can add custom styles if needed
}

// Initialize country selector
function initializeCountrySelector() {
    const countrySelect = $('#country');

    ALLOWED_COUNTRY_CODES.forEach(code => {
        countrySelect.append($('<option>', {value: code}).text(regionName(code)));
    });
    
    // Update flag when country changes
    countrySelect.on('change', function() {
        const selectedCode = $(this).val();
        const flagPath = selectedCode ? `flags/${selectedCode}.svg` : 'flags/unknown.svg';
        $('#selectedFlag').attr('src', flagPath);
    });
    
    // Try to detect user's country from browser
    detectUserCountry();
}

// Detect user's country from browser locale
function detectUserCountry() {
    const userLang = navigator.language || navigator.userLanguage;
    const countryCode = userLang.split('-')[1]?.toLowerCase();
    
    if (countryCode && ALLOWED_COUNTRY_CODES.includes(countryCode)) {
        $('#country').val(countryCode).trigger('change');
    }
}

// Load teasers (with fallback to placeholders)
async function loadTeasers() {
    try {
        const response = await $.get(getLtsUrl('teasers'));
        if (response.teasers != null) {
             response.teasers.forEach(teaser => {
                  teaser.image = getLtsUrl(teaser.image.replace(/^\/?signup-api\//, ""));
            });
        }
        currentTeasers = response.teasers || PLACEHOLDER_TEASERS;
        displayTeasers();
    } catch (error) {
        // translatable: false — "Failed to load teasers:" is a console diagnostic, never rendered
        console.error('Failed to load teasers:', error);
        currentTeasers = PLACEHOLDER_TEASERS;
        displayTeasers();
    }
}

// Display teaser cards in carousel
function displayTeasers() {
    const indicatorsContainer = $('#carouselIndicators');
    const innerContainer = $('#carouselInner');
    
    indicatorsContainer.empty();
    innerContainer.empty();
    
    currentTeasers.sort((a, b) => (a.order || 0) - (b.order || 0));
    
    // Group teasers into slides (3 per slide on desktop, 1 on mobile)
    const itemsPerSlide = window.innerWidth >= 768 ? 3 : 1;
    const slides = [];
    
    for (let i = 0; i < currentTeasers.length; i += itemsPerSlide) {
        slides.push(currentTeasers.slice(i, i + itemsPerSlide));
    }
    
    // Create carousel slides
    slides.forEach((slideItems, slideIndex) => {
        // Add indicator
        // translatable: false — 'class="active" aria-current="true"' is the indicator attribute pair the carousel reads, not copy
        const indicator = `<button type="button" data-bs-target="#teaserCarousel" data-bs-slide-to="${slideIndex}" 
                          ${slideIndex === 0 ? 'class="active" aria-current="true"' : ''} 
                          aria-label="${tAttr("trial.carousel.slide", {number: slideIndex + 1})}"></button>`;
        indicatorsContainer.append(indicator);
        
        // Create slide
        const slideCards = slideItems.map(teaser => {
            const featuresHtml = teaser.features ? 
                `<ul class="list-unstyled">${teaser.features.map(f => `<li><i class="fas fa-check text-success"></i> ${f}</li>`).join('')}</ul>` : '';
                
            const ctaText = teaser.ctaText || t("trial.teaser.cta");
            const imageSrc = teaser.image || teaser.imageUrl;
                
            return `
                <div class="col-md-4">
                    <div class="card h-100 shadow-sm teaser-card">
                        <img src="${imageSrc}" class="card-img-top" alt="${teaser.title}" style="height: 200px; object-fit: contain; background-color: #f8f9fa;">
                        <div class="card-body d-flex flex-column">
                            <h5 class="card-title">${teaser.title}</h5>
                            <p class="card-text">${teaser.description}</p>
                            ${featuresHtml}
                            <div class="mt-auto">
                                <button class="btn btn-outline-primary w-100 btn-teaser-cta" data-action="signup">
                                    <i class="fas fa-arrow-right"></i> ${ctaText}
                                </button>
                            </div>
                        </div>
                    </div>
                </div>
            `;
        }).join('');
        
        const slide = `
            <div class="carousel-item ${slideIndex === 0 ? 'active' : ''}">
                <div class="row g-4">
                    ${slideCards}
                </div>
            </div>
        `;
        
        innerContainer.append(slide);
    });
    
    // Add click handlers to CTA buttons
    $('.btn-teaser-cta').on('click', function() {
        redirectToTrialSetup();
    });
    
    // Reinitialize carousel if needed
    if (slides.length > 1) {
        $('#teaserCarousel').carousel();
    } else {
        // Hide controls if only one slide
        // translatable: false — '.carousel-control-prev, .carousel-control-next, .carousel-indicators' is a DOM selector
        $('.carousel-control-prev, .carousel-control-next, .carousel-indicators').hide();
    }
}

// Helper function to construct full LTS URLs
function getLtsUrl(endpoint) {
    return buildSignupApiUrl(ltsBaseUrl, endpoint);
}

// Fetch node ID and LTS URL from configuration
async function fetchNodeId() {
    try {
        const response = await sendWsRequest("LtsTrialConfigResult", { LtsTrialConfig: {} });
        const data = response && response.data ? response.data : {};
        nodeId = data.node_id || null;
        
        // Extract LTS URL from config, defaulting to the standard URL
        if (data.lts_url) {
            ltsBaseUrl = data.lts_url;
            // Ensure the URL ends with a slash
            if (!ltsBaseUrl.endsWith('/')) {
                ltsBaseUrl += '/';
            }
        }
    } catch (error) {
        // translatable: false — "Failed to fetch node ID:" is a console diagnostic, never rendered
        console.error('Failed to fetch node ID:', error);
        nodeId = null;
    }
}

// Fetch circuit count
async function fetchCircuitCount() {
    try {
        const response = await sendWsRequest("CircuitCountResult", { CircuitCount: {} });
        const data = response && response.data ? response.data : {};
        const count = data.count || 0;
        const configuredCount = data.configured_count || 0;
        
        // Always show circuit count if we have any circuits (active or configured)
        if (count > 0 || configuredCount > 0) {
            // If using configured count (no active circuits), show with note
            const displayCount = count || configuredCount;
            const isConfigured = count === 0 && configuredCount > 0;

            $('#circuitNumber').text(formatNumber(displayCount));
            $('#circuitState').text(isConfigured
                ? tn("trial.count.configured_circuits", displayCount)
                : tn("trial.count.active_circuits", displayCount));
            const monthlyPrice = (displayCount * 0.30).toFixed(2);
            $('#monthlyPrice').text(monthlyPrice);
            $('#circuitCount').fadeIn();
        }
    } catch (error) {
        // translatable: false — "Failed to fetch circuit count:" is a console diagnostic, never rendered
        console.error('Failed to fetch circuit count:', error);
    }
}

// Show/hide sections
function showSection(sectionId) {
    const sections = ['teaserSection', 'licenseKeySection', 'licenseRecoverySection', 'signupSection', 'successSection'];
    sections.forEach(id => {
        $(`#${id}`).hide();
    });
    $(`#${sectionId}`).fadeIn();
}

function redirectToTrialSetup() {
    window.location.href = 'lts_trial_setup.html';
}

function openInitialViewFromQuery() {
    const params = new URLSearchParams(window.location.search);
    const view = params.get('view');

    if (view === 'signup') {
        showSection('signupSection');
        return;
    }

    if (view === 'license') {
        showSection('licenseKeySection');
        return;
    }

    if (view === 'recover') {
        showSection('licenseRecoverySection');
    }
}

// Show loading modal
function showLoading(message = t("trial.progress.processing")) {
    $('#loadingMessage').text(message);
    $('#creatingAccountModal').modal('show');
}

// Hide loading modal
function hideLoading() {
    console.log('[hideLoading] Hiding creatingAccountModal at', new Date().toISOString());
    $('#creatingAccountModal').modal('hide');
    // Force remove modal backdrop and modal-open class
    setTimeout(() => {
        $('.modal-backdrop').remove();
        $('body').removeClass('modal-open');
        $('#creatingAccountModal').removeClass('show').hide();
    }, 100);
}

// Show error modal
function showError(message) {
    console.log('[showError] Called with message:', message, 'at', new Date().toISOString());
    // Ensure any loading modal is fully hidden first
    $('#creatingAccountModal').modal('hide');

    // Force remove lingering modal-backdrop and modal-open class after 200ms
    setTimeout(() => {
        if ($('#creatingAccountModal').hasClass('show')) {
            console.log('[showError] Forcibly removing creatingAccountModal (still visible) at', new Date().toISOString());
            $('#creatingAccountModal').removeClass('show').hide();
        }
        // Remove any lingering backdrops
        $('.modal-backdrop').remove();
        $('body').removeClass('modal-open');
        console.log('[showError] Removed modal-backdrop and modal-open class at', new Date().toISOString());
        $('#errorMessage').text(message);
        $('#errorModal').modal('show');
    }, 200);
}

// Validate license key format
function validateLicenseKeyFormat(key) {
    // Accept any format with hexadecimal characters (0-9, A-F) and dashes
    // Must have at least one dash and be between 16-40 characters total
    const pattern = /^[A-F0-9\-]{16,40}$/;
    const upperKey = key.toUpperCase();
    // Ensure it has at least one dash and some hex characters
    return pattern.test(upperKey) && upperKey.includes('-') && /[A-F0-9]/.test(upperKey);
}

// Attach event handlers
function attachEventHandlers() {
    // Navigation buttons
    $('#btnExistingCustomer').on('click', () => {
        // Analytics tracking for existing customer path
        const trackingImg = new Image();
        // translatable: false — "&type=existing" is a query-string parameter the signup ping records
        trackingImg.src = getLtsUrl('signupPing') + '?t=' + Date.now() + '&type=existing';
        showSection('licenseKeySection');
    });
    $('#btnNewCustomer').on('click', () => {
        // Analytics tracking for new customer path
        const trackingImg = new Image();
        // translatable: false — "&type=new" is a query-string parameter the signup ping records
        trackingImg.src = getLtsUrl('signupPing') + '?t=' + Date.now() + '&type=new';
        redirectToTrialSetup();
    });
    $('#btnBackFromLicense').on('click', () => showSection('teaserSection'));
    $('#btnBackFromSignup').on('click', () => showSection('teaserSection'));
    $('#btnForgotLicense').on('click', () => showSection('licenseRecoverySection'));
    $('#btnBackFromRecovery').on('click', () => showSection('licenseKeySection'));
    $('#btnDashboard').on('click', () => window.location.href = '/');
    
    // License key form
    $('#licenseKeyForm').on('submit', async function(e) {
        e.preventDefault();
        
        const licenseKey = $('#licenseKey').val().trim().toUpperCase();
        
        if (!validateLicenseKeyFormat(licenseKey)) {
            $('#licenseKey').addClass('is-invalid');
            return;
        }
        
        $('#licenseKey').removeClass('is-invalid');
        showLoading(t("trial.license.validating"));
        
        try {
            const response = await $.ajax({
                url: getLtsUrl('validateLicense'),
                method: 'POST',
                contentType: 'application/json',
                data: JSON.stringify({ license_key: licenseKey })
            });
            
            if (response.valid) {
                // Show success with the validated license key
                hideLoading();
                showSection('successSection');
                $('#successMessage').html(`
                    <strong>${t("trial.success.key_label")}</strong> ${escapeHtml(licenseKey)}<br>
                    <small>${t("trial.success.validated")}</small>
                `);
                $('#configStatus').text(t("trial.success.saving"));

                wsClient.send({ LtsSignUp: { license_key: licenseKey } });
                // Show success message now, then swap spinner to a check and enable dashboard after 5s
                $('#configStatus').text(t("trial.success.license_applied"));
                setTimeout(() => {
                    $('#configSpinner').hide();
                    $('#configStatus').html(`<i class="fas fa-check-circle text-success"></i> ${t("trial.success.saved")}`);
                    $('#btnDashboard').fadeIn();
                }, 5000);
            } else {
                hideLoading();
                showError(response.message || t("trial.license.rejected"));
            }
        } catch (error) {
            hideLoading();
            showError(t("trial.license.validate_failed"));
        }
    });
    
    // License recovery form
    $('#licenseRecoveryForm').on('submit', async function(e) {
        e.preventDefault();
        
        const email = $('#recoveryEmail').val().trim();
        
        if (!email || !email.includes('@')) {
            $('#recoveryEmail').addClass('is-invalid');
            return;
        }
        
        $('#recoveryEmail').removeClass('is-invalid');
        showLoading(t("trial.recovery.sending"));
        
        try {
            await $.ajax({
                url: getLtsUrl('recoverLicense'),
                method: 'POST',
                contentType: 'application/json',
                data: JSON.stringify({ email })
            });
            
            hideLoading();
            showError(t("trial.recovery.sent"));
            $('#recoveryEmail').val('');
            showSection('licenseKeySection');
        } catch (error) {
            hideLoading();
            showError(t("trial.recovery.send_failed"));
        }
    });
    
    // Signup form
    $('#signupForm').on('submit', async function(e) {
        e.preventDefault();
        console.log('[signupForm] Submit handler triggered at', new Date().toISOString());

        const form = this;
        if (!form.checkValidity()) {
            e.stopPropagation();
            form.classList.add('was-validated');
            return;
        }

        form.classList.add('was-validated');

        // Analytics tracking pixel
        const trackingImg = new Image();
        trackingImg.src = getLtsUrl('signupPing') + '?t=' + Date.now();

        const formData = {
            nodeId: nodeId || 'unknown',
            name: $('#customerName').val().trim(),
            email: $('#customerEmail').val().trim(),
            business_name: $('#businessName').val().trim(),
            address1: $('#address1').val().trim(),
            address2: $('#address2').val().trim(),
            city: $('#city').val().trim(),
            state: $('#state').val().trim(),
            zip: $('#zip').val().trim(),
            country: $('#country').val(),
            phone: $('#phone').val().trim(),
            website: $('#website').val().trim()
        };

        showLoading(t("trial.signup.creating"));
        console.log('[signupForm] showLoading called at', new Date().toISOString());

        try {
            const response = await $.ajax({
                url: getLtsUrl('signupCustomer'),
                method: 'POST',
                contentType: 'application/json',
                data: JSON.stringify(formData)
            });

            console.log('[signupForm] AJAX success at', new Date().toISOString());
            
            if (response.licenseKey) {
                // Show success with the license key
                hideLoading();
                showSection('successSection');
                $('#successMessage').html(`
                    <strong>${t("trial.success.key_label")}</strong> ${escapeHtml(response.licenseKey)}<br>
                    <small>${t("trial.success.created_detail")}</small>
                `);
                $('#configStatus').text(t("trial.success.saving"));

                wsClient.send({ LtsSignUp: { license_key: response.licenseKey } });
                // Show success message now, then swap spinner to a check and enable dashboard after 5s
                $('#configStatus').text(t("trial.success.created"));
                setTimeout(() => {
                    $('#configSpinner').hide();
                    $('#configStatus').html(`<i class="fas fa-check-circle text-success"></i> ${t("trial.success.saved")}`);
                    $('#btnDashboard').fadeIn();
                }, 5000);
            } else {
                console.log('[signupForm] No licenseKey in response, error at', new Date().toISOString());
                showError(t("trial.signup.create_failed"));
            }
        } catch (error) {
            console.log('[signupForm] AJAX error at', new Date().toISOString(), error);
            hideLoading();
            showError(t("trial.signup.create_failed_detail"));
        }
    });
}
