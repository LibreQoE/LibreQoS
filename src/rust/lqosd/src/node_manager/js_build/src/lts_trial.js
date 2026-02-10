import {PLACEHOLDER_TEASERS} from "./lts_teasers_shared";
import {get_ws_client} from "./pubsub/ws";

const wsClient = get_ws_client();
const listenOnce = (eventName, handler) => {
    const wrapped = (msg) => {
        wsClient.off(eventName, wrapped);
        handler(msg);
    };
    wsClient.on(eventName, wrapped);
};

function sendWsRequest(responseEvent, request) {
    return new Promise((resolve, reject) => {
        let done = false;
        const onResponse = (msg) => {
            if (done) return;
            done = true;
            wsClient.off(responseEvent, onResponse);
            wsClient.off("Error", onError);
            resolve(msg);
        };
        const onError = (msg) => {
            if (done) return;
            done = true;
            wsClient.off(responseEvent, onResponse);
            wsClient.off("Error", onError);
            reject(msg);
        };
        wsClient.on(responseEvent, onResponse);
        wsClient.on("Error", onError);
        wsClient.send(request);
    });
}

// Paddle-compatible countries (excluding embargoed nations)
const ALLOWED_COUNTRIES = [
    { code: 'ad', name: 'Andorra' },
    { code: 'ae', name: 'United Arab Emirates' },
    { code: 'ag', name: 'Antigua and Barbuda' },
    { code: 'ai', name: 'Anguilla' },
    { code: 'al', name: 'Albania' },
    { code: 'am', name: 'Armenia' },
    { code: 'ao', name: 'Angola' },
    { code: 'ar', name: 'Argentina' },
    { code: 'at', name: 'Austria' },
    { code: 'au', name: 'Australia' },
    { code: 'aw', name: 'Aruba' },
    { code: 'ax', name: 'Åland Islands' },
    { code: 'az', name: 'Azerbaijan' },
    { code: 'ba', name: 'Bosnia and Herzegovina' },
    { code: 'bb', name: 'Barbados' },
    { code: 'bd', name: 'Bangladesh' },
    { code: 'be', name: 'Belgium' },
    { code: 'bf', name: 'Burkina Faso' },
    { code: 'bg', name: 'Bulgaria' },
    { code: 'bh', name: 'Bahrain' },
    { code: 'bi', name: 'Burundi' },
    { code: 'bj', name: 'Benin' },
    { code: 'bl', name: 'Saint Barthélemy' },
    { code: 'bm', name: 'Bermuda' },
    { code: 'bn', name: 'Brunei' },
    { code: 'bo', name: 'Bolivia' },
    { code: 'bq', name: 'Caribbean Netherlands' },
    { code: 'br', name: 'Brazil' },
    { code: 'bs', name: 'Bahamas' },
    { code: 'bt', name: 'Bhutan' },
    { code: 'bw', name: 'Botswana' },
    { code: 'bz', name: 'Belize' },
    { code: 'ca', name: 'Canada' },
    { code: 'cc', name: 'Cocos (Keeling) Islands' },
    { code: 'cd', name: 'Congo (DRC)' },
    { code: 'cf', name: 'Central African Republic' },
    { code: 'cg', name: 'Congo' },
    { code: 'ch', name: 'Switzerland' },
    { code: 'ci', name: 'Côte d\'Ivoire' },
    { code: 'ck', name: 'Cook Islands' },
    { code: 'cl', name: 'Chile' },
    { code: 'cm', name: 'Cameroon' },
    { code: 'cn', name: 'China' },
    { code: 'co', name: 'Colombia' },
    { code: 'cr', name: 'Costa Rica' },
    { code: 'cv', name: 'Cape Verde' },
    { code: 'cw', name: 'Curaçao' },
    { code: 'cx', name: 'Christmas Island' },
    { code: 'cy', name: 'Cyprus' },
    { code: 'cz', name: 'Czech Republic' },
    { code: 'de', name: 'Germany' },
    { code: 'dj', name: 'Djibouti' },
    { code: 'dk', name: 'Denmark' },
    { code: 'dm', name: 'Dominica' },
    { code: 'do', name: 'Dominican Republic' },
    { code: 'dz', name: 'Algeria' },
    { code: 'ec', name: 'Ecuador' },
    { code: 'ee', name: 'Estonia' },
    { code: 'eg', name: 'Egypt' },
    { code: 'er', name: 'Eritrea' },
    { code: 'es', name: 'Spain' },
    { code: 'et', name: 'Ethiopia' },
    { code: 'fi', name: 'Finland' },
    { code: 'fj', name: 'Fiji' },
    { code: 'fk', name: 'Falkland Islands' },
    { code: 'fm', name: 'Micronesia' },
    { code: 'fo', name: 'Faroe Islands' },
    { code: 'fr', name: 'France' },
    { code: 'ga', name: 'Gabon' },
    { code: 'gb', name: 'United Kingdom' },
    { code: 'gd', name: 'Grenada' },
    { code: 'ge', name: 'Georgia' },
    { code: 'gf', name: 'French Guiana' },
    { code: 'gg', name: 'Guernsey' },
    { code: 'gh', name: 'Ghana' },
    { code: 'gi', name: 'Gibraltar' },
    { code: 'gl', name: 'Greenland' },
    { code: 'gm', name: 'Gambia' },
    { code: 'gn', name: 'Guinea' },
    { code: 'gp', name: 'Guadeloupe' },
    { code: 'gq', name: 'Equatorial Guinea' },
    { code: 'gr', name: 'Greece' },
    { code: 'gs', name: 'South Georgia' },
    { code: 'gt', name: 'Guatemala' },
    { code: 'gu', name: 'Guam' },
    { code: 'gw', name: 'Guinea-Bissau' },
    { code: 'gy', name: 'Guyana' },
    { code: 'hk', name: 'Hong Kong' },
    { code: 'hm', name: 'Heard Island' },
    { code: 'hn', name: 'Honduras' },
    { code: 'hr', name: 'Croatia' },
    { code: 'ht', name: 'Haiti' },
    { code: 'hu', name: 'Hungary' },
    { code: 'id', name: 'Indonesia' },
    { code: 'ie', name: 'Ireland' },
    { code: 'il', name: 'Israel' },
    { code: 'im', name: 'Isle of Man' },
    { code: 'in', name: 'India' },
    { code: 'io', name: 'British Indian Ocean Territory' },
    { code: 'iq', name: 'Iraq' },
    { code: 'is', name: 'Iceland' },
    { code: 'it', name: 'Italy' },
    { code: 'je', name: 'Jersey' },
    { code: 'jm', name: 'Jamaica' },
    { code: 'jo', name: 'Jordan' },
    { code: 'jp', name: 'Japan' },
    { code: 'ke', name: 'Kenya' },
    { code: 'kg', name: 'Kyrgyzstan' },
    { code: 'kh', name: 'Cambodia' },
    { code: 'ki', name: 'Kiribati' },
    { code: 'km', name: 'Comoros' },
    { code: 'kn', name: 'Saint Kitts and Nevis' },
    { code: 'kr', name: 'South Korea' },
    { code: 'kw', name: 'Kuwait' },
    { code: 'ky', name: 'Cayman Islands' },
    { code: 'kz', name: 'Kazakhstan' },
    { code: 'la', name: 'Laos' },
    { code: 'lb', name: 'Lebanon' },
    { code: 'lc', name: 'Saint Lucia' },
    { code: 'li', name: 'Liechtenstein' },
    { code: 'lk', name: 'Sri Lanka' },
    { code: 'lr', name: 'Liberia' },
    { code: 'ls', name: 'Lesotho' },
    { code: 'lt', name: 'Lithuania' },
    { code: 'lu', name: 'Luxembourg' },
    { code: 'lv', name: 'Latvia' },
    { code: 'ly', name: 'Libya' },
    { code: 'ma', name: 'Morocco' },
    { code: 'mc', name: 'Monaco' },
    { code: 'md', name: 'Moldova' },
    { code: 'me', name: 'Montenegro' },
    { code: 'mf', name: 'Saint Martin' },
    { code: 'mg', name: 'Madagascar' },
    { code: 'mh', name: 'Marshall Islands' },
    { code: 'mk', name: 'North Macedonia' },
    { code: 'ml', name: 'Mali' },
    { code: 'mm', name: 'Myanmar' },
    { code: 'mn', name: 'Mongolia' },
    { code: 'mo', name: 'Macao' },
    { code: 'mp', name: 'Northern Mariana Islands' },
    { code: 'mq', name: 'Martinique' },
    { code: 'mr', name: 'Mauritania' },
    { code: 'ms', name: 'Montserrat' },
    { code: 'mt', name: 'Malta' },
    { code: 'mu', name: 'Mauritius' },
    { code: 'mv', name: 'Maldives' },
    { code: 'mw', name: 'Malawi' },
    { code: 'mx', name: 'Mexico' },
    { code: 'my', name: 'Malaysia' },
    { code: 'mz', name: 'Mozambique' },
    { code: 'na', name: 'Namibia' },
    { code: 'nc', name: 'New Caledonia' },
    { code: 'ne', name: 'Niger' },
    { code: 'nf', name: 'Norfolk Island' },
    { code: 'ng', name: 'Nigeria' },
    { code: 'ni', name: 'Nicaragua' },
    { code: 'nl', name: 'Netherlands' },
    { code: 'no', name: 'Norway' },
    { code: 'np', name: 'Nepal' },
    { code: 'nr', name: 'Nauru' },
    { code: 'nu', name: 'Niue' },
    { code: 'nz', name: 'New Zealand' },
    { code: 'om', name: 'Oman' },
    { code: 'pa', name: 'Panama' },
    { code: 'pe', name: 'Peru' },
    { code: 'pf', name: 'French Polynesia' },
    { code: 'pg', name: 'Papua New Guinea' },
    { code: 'ph', name: 'Philippines' },
    { code: 'pk', name: 'Pakistan' },
    { code: 'pl', name: 'Poland' },
    { code: 'pm', name: 'Saint Pierre and Miquelon' },
    { code: 'pn', name: 'Pitcairn Islands' },
    { code: 'pr', name: 'Puerto Rico' },
    { code: 'ps', name: 'Palestine' },
    { code: 'pt', name: 'Portugal' },
    { code: 'pw', name: 'Palau' },
    { code: 'py', name: 'Paraguay' },
    { code: 'qa', name: 'Qatar' },
    { code: 're', name: 'Réunion' },
    { code: 'ro', name: 'Romania' },
    { code: 'rs', name: 'Serbia' },
    { code: 'rw', name: 'Rwanda' },
    { code: 'sa', name: 'Saudi Arabia' },
    { code: 'sb', name: 'Solomon Islands' },
    { code: 'sc', name: 'Seychelles' },
    { code: 'se', name: 'Sweden' },
    { code: 'sg', name: 'Singapore' },
    { code: 'sh', name: 'Saint Helena' },
    { code: 'si', name: 'Slovenia' },
    { code: 'sj', name: 'Svalbard and Jan Mayen' },
    { code: 'sk', name: 'Slovakia' },
    { code: 'sl', name: 'Sierra Leone' },
    { code: 'sm', name: 'San Marino' },
    { code: 'sn', name: 'Senegal' },
    { code: 'so', name: 'Somalia' },
    { code: 'sr', name: 'Suriname' },
    { code: 'ss', name: 'South Sudan' },
    { code: 'st', name: 'São Tomé and Príncipe' },
    { code: 'sv', name: 'El Salvador' },
    { code: 'sx', name: 'Sint Maarten' },
    { code: 'sz', name: 'Eswatini' },
    { code: 'tc', name: 'Turks and Caicos Islands' },
    { code: 'td', name: 'Chad' },
    { code: 'tf', name: 'French Southern Territories' },
    { code: 'tg', name: 'Togo' },
    { code: 'th', name: 'Thailand' },
    { code: 'tj', name: 'Tajikistan' },
    { code: 'tk', name: 'Tokelau' },
    { code: 'tl', name: 'Timor-Leste' },
    { code: 'tm', name: 'Turkmenistan' },
    { code: 'tn', name: 'Tunisia' },
    { code: 'to', name: 'Tonga' },
    { code: 'tr', name: 'Turkey' },
    { code: 'tt', name: 'Trinidad and Tobago' },
    { code: 'tv', name: 'Tuvalu' },
    { code: 'tw', name: 'Taiwan' },
    { code: 'tz', name: 'Tanzania' },
    { code: 'ua', name: 'Ukraine' },
    { code: 'ug', name: 'Uganda' },
    { code: 'um', name: 'U.S. Outlying Islands' },
    { code: 'us', name: 'United States' },
    { code: 'uy', name: 'Uruguay' },
    { code: 'uz', name: 'Uzbekistan' },
    { code: 'va', name: 'Vatican City' },
    { code: 'vc', name: 'Saint Vincent and the Grenadines' },
    { code: 've', name: 'Venezuela' },
    { code: 'vg', name: 'British Virgin Islands' },
    { code: 'vi', name: 'U.S. Virgin Islands' },
    { code: 'vn', name: 'Vietnam' },
    { code: 'vu', name: 'Vanuatu' },
    { code: 'wf', name: 'Wallis and Futuna' },
    { code: 'ws', name: 'Samoa' },
    { code: 'ye', name: 'Yemen' },
    { code: 'yt', name: 'Mayotte' },
    { code: 'za', name: 'South Africa' },
    { code: 'zm', name: 'Zambia' },
    { code: 'zw', name: 'Zimbabwe' }
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
    
    ALLOWED_COUNTRIES.forEach(country => {
        countrySelect.append(`<option value="${country.code}">${country.name}</option>`);
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
    
    if (countryCode && ALLOWED_COUNTRIES.find(c => c.code === countryCode)) {
        $('#country').val(countryCode).trigger('change');
    }
}

// Load teasers (with fallback to placeholders)
async function loadTeasers() {
    try {
        const response = await $.get(getLtsUrl('teasers'));
        if (response.teasers != null) {
             response.teasers.forEach(teaser => {
                  teaser.image = getLtsUrl(teaser.image.replace("signup-api/", ""));
            });
        }
        currentTeasers = response.teasers || PLACEHOLDER_TEASERS;
        displayTeasers();
    } catch (error) {
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
        const indicator = `<button type="button" data-bs-target="#teaserCarousel" data-bs-slide-to="${slideIndex}" 
                          ${slideIndex === 0 ? 'class="active" aria-current="true"' : ''} 
                          aria-label="Slide ${slideIndex + 1}"></button>`;
        indicatorsContainer.append(indicator);
        
        // Create slide
        const slideCards = slideItems.map(teaser => {
            const featuresHtml = teaser.features ? 
                `<ul class="list-unstyled">${teaser.features.map(f => `<li><i class="fas fa-check text-success"></i> ${f}</li>`).join('')}</ul>` : '';
                
            const ctaText = teaser.ctaText || 'Try Insight Free';
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
        showSection('signupSection');
    });
    
    // Reinitialize carousel if needed
    if (slides.length > 1) {
        $('#teaserCarousel').carousel();
    } else {
        // Hide controls if only one slide
        $('.carousel-control-prev, .carousel-control-next, .carousel-indicators').hide();
    }
}

// Helper function to construct full LTS URLs
function getLtsUrl(endpoint) {
    // Ensure ltsBaseUrl starts with http:// or https://, otherwise prefix with https://
    let baseUrl = ltsBaseUrl;
    if (!/^https?:\/\//i.test(baseUrl)) {
        baseUrl = 'https://' + baseUrl;
    }
    // Ensure baseUrl ends with a single slash
    let base = baseUrl.endsWith('/') ? baseUrl : baseUrl + '/';
    // Ensure 'signup-api/' is appended exactly once
    base += base.endsWith('signup-api/') ? '' : 'signup-api/';
    // Remove any leading slash from endpoint
    endpoint = endpoint.replace(/^\/+/, '');
    //console.log("Base: ", base, "Endpoint:", endpoint);
    return base + endpoint;
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
            
            $('#circuitNumber').text(displayCount.toLocaleString() + (isConfigured ? ' Configured' : ' Active'));
            const monthlyPrice = (displayCount * 0.30).toFixed(2);
            $('#monthlyPrice').text(monthlyPrice);
            $('#circuitCount').fadeIn();
        }
    } catch (error) {
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

// Show loading modal
function showLoading(message = 'Processing...') {
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
        trackingImg.src = getLtsUrl('signupPing') + '?t=' + Date.now() + '&type=existing';
        showSection('licenseKeySection');
    });
    $('#btnNewCustomer').on('click', () => {
        // Analytics tracking for new customer path
        const trackingImg = new Image();
        trackingImg.src = getLtsUrl('signupPing') + '?t=' + Date.now() + '&type=new';
        showSection('signupSection');
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
        showLoading('Validating license key...');
        
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
                    <strong>License Key:</strong> ${licenseKey}<br>
                    <small>Your license has been validated.</small>
                `);
                $('#configStatus').text('Saving configuration...');

                wsClient.send({ LtsSignUp: { license_key: licenseKey } });
                // Show success message now, then swap spinner to a check and enable dashboard after 5s
                $('#configStatus').html(`License validated! Your configuration has been updated - data will start going to Insight shortly.`);
                setTimeout(() => {
                    $('#configSpinner').hide();
                    $('#configStatus').html(`<i class="fas fa-check-circle text-success"></i> Configuration saved.`);
                    $('#btnDashboard').fadeIn();
                }, 5000);
            } else {
                hideLoading();
                showError(response.message || 'Invalid license key.');
            }
        } catch (error) {
            hideLoading();
            showError('Failed to validate license key. Please try again.');
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
        showLoading('Sending recovery email...');
        
        try {
            await $.ajax({
                url: getLtsUrl('recoverLicense'),
                method: 'POST',
                contentType: 'application/json',
                data: JSON.stringify({ email })
            });
            
            hideLoading();
            showError('If a license key exists for this email, you will receive it shortly.');
            $('#recoveryEmail').val('');
            showSection('licenseKeySection');
        } catch (error) {
            hideLoading();
            showError('Failed to send recovery email. Please try again.');
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

        showLoading('Creating your account...');
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
                    <strong>License Key:</strong> ${response.licenseKey}<br>
                    <small>An email with your license key and portal access has been sent to your email address.</small>
                `);
                $('#configStatus').text('Saving configuration...');

                wsClient.send({ LtsSignUp: { license_key: response.licenseKey } });
                // Show success message now, then swap spinner to a check and enable dashboard after 5s
                $('#configStatus').html(`Account created! Your configuration has been updated - data will start going to Insight shortly.`);
                setTimeout(() => {
                    $('#configSpinner').hide();
                    $('#configStatus').html(`<i class="fas fa-check-circle text-success"></i> Configuration saved.`);
                    $('#btnDashboard').fadeIn();
                }, 5000);
            } else {
                console.log('[signupForm] No licenseKey in response, error at', new Date().toISOString());
                showError('Failed to create account. Please try again.');
            }
        } catch (error) {
            console.log('[signupForm] AJAX error at', new Date().toISOString(), error);
            hideLoading();
            showError('Failed to create account. Please check your information and try again.');
        }
    });
}
