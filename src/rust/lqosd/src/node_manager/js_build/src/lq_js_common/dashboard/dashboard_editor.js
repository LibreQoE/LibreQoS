/**
 * Opens a full screen modal that lets the user edit dashboard items with tab support.
 * @param {Object} layout - The current dashboard layout object with tabs
 * @param {Array} availableElements - Array of objects representing available elements to add (each with a "name" and default "size").
 * @param {Function} callback - A function that will be called with the new layout when the user clicks "Done".
 * @param {String} cookieName - The localStorage key for saving the dashboard layout.
 */
export function openDashboardEditor(layout, availableElements, callback, cookieName) {
    // Keep track of current tab and layout state
    let currentLayout = JSON.parse(JSON.stringify(layout)); // Deep copy
    let activeTabIndex = currentLayout.activeTab || 0;
    
    // Create a lookup map for widget names by tag
    const widgetNameLookup = {};
    availableElements.forEach(element => {
        widgetNameLookup[element.tag] = element.name;
    });
    
    // Build modal HTML with tab support
    var modalHtml = `
  <style>
    .border-dashed {
        border: 2px dashed #dee2e6 !important;
    }
    .card.h-100 {
        min-height: 100px;
    }
    .tab-manager {
        display: flex;
        align-items: center;
        gap: 10px;
        margin-bottom: 15px;
    }
    .tab-manager .nav-tabs {
        flex: 1;
        margin-bottom: 0;
    }
    .dashboard-grid-container {
        min-height: 400px;
    }
    .nav-link:hover .fa-pencil-alt {
        opacity: 1 !important;
    }
    .tab-name:focus + .fa-pencil-alt {
        display: none;
    }
    /* Make drop overlay non-interactive so it doesn't block drags */
    #dropZone { pointer-events: none; }
  </style>
  <div class="modal fade" id="dashboardEditorModal" tabindex="-1" aria-labelledby="dashboardEditorModalLabel" aria-hidden="true">
    <div class="modal-dialog modal-fullscreen">
      <div class="modal-content">
        <div class="modal-header">
          <h5 class="modal-title" id="dashboardEditorModalLabel">Dashboard Editor</h5>
          <button type="button" class="btn btn-primary btn-sm ms-3" id="downloadLayoutButton" title="Save layout to file">
            <i class="fas fa-download me-1"></i>Download Layout
          </button>
          <button type="button" class="btn btn-success btn-sm ms-2" id="uploadLayoutButton" title="Load layout from file">
            <i class="fas fa-upload me-1"></i>Upload Layout
          </button>
          <input type="file" id="layoutFileInput" accept=".json" style="display: none;">
          <button type="button" class="btn btn-danger btn-sm ms-3" id="clearAllButton">
            <i class="fas fa-trash me-1"></i>Clear Current Tab
          </button>
          <button type="button" class="btn btn-warning btn-sm ms-2" id="restoreDefaultsButton">
            <i class="fas fa-undo me-1"></i>Restore Defaults
          </button>
          <button type="button" class="btn-close" data-bs-dismiss="modal" aria-label="Close"></button>
        </div>
        <div class="modal-body">
          <div id="dropZone" class="d-none position-absolute top-0 start-0 w-100 h-100 
               bg-primary bg-opacity-10 d-flex align-items-center justify-content-center"
               style="z-index: 1000;">
              <div class="text-center">
                  <i class="fas fa-file-upload fa-4x text-primary mb-3"></i>
                  <h4>Drop layout file here</h4>
              </div>
          </div>
          <div class="container-fluid">
            <div class="row">
              <!-- Main grid area for dashboard items -->
              <div class="col-md-9">
                <div class="tab-manager">
                  <ul class="nav nav-tabs" id="editorTabs">
                    <!-- Tabs will be injected here -->
                  </ul>
                  <button type="button" class="btn btn-sm btn-primary" id="addTabButton">
                    <i class="fas fa-plus"></i> Add Tab
                  </button>
                </div>
                <div class="dashboard-grid-container">
                  <div id="dashboardGrid" class="row gy-3">
                    <!-- Dashboard items will be injected here -->
                  </div>
                </div>
              </div>
              <!-- Panel for available elements -->
              <div class="col-md-3 border-start">
                <ul id="availableList" class="list-group">
                  <!-- Available elements will be injected here -->
                </ul>
              </div>
            </div>
          </div>
        </div>
        <div class="modal-footer">
          <button id="dashboardDone" type="button" class="btn btn-primary">Done</button>
          <button type="button" class="btn btn-secondary" data-bs-dismiss="modal">Cancel</button>
        </div>
      </div>
    </div>
  </div>
  `;

    // Append the modal HTML to the body.
    $('body').append(modalHtml);

    // Helper to show alerts
    function showAlert(message, type = 'info') {
        const alert = $(`<div class="alert alert-${type} alert-dismissible fade show" role="alert">
            ${message}
            <button type="button" class="btn-close" data-bs-dismiss="alert" aria-label="Close"></button>
        </div>`);
        $('.modal-body .container-fluid').prepend(alert);
        setTimeout(() => alert.alert('close'), 4000);
    }

    // Drag & Drop overlay handlers
    const dropZone = document.getElementById('dropZone');
    ['dragenter', 'dragover'].forEach(evtName => {
        document.addEventListener(evtName, (e) => {
            e.preventDefault();
            e.stopPropagation();
            dropZone.classList.remove('d-none');
        });
    });
    ;['dragleave', 'drop'].forEach(evtName => {
        document.addEventListener(evtName, (e) => {
            e.preventDefault();
            e.stopPropagation();
            dropZone.classList.add('d-none');
        });
    });
    document.addEventListener('drop', (e) => {
        const dt = e.dataTransfer;
        if (dt && dt.files && dt.files.length) {
            uploadLayout(dt.files[0]);
        }
    });

    function renderTabs() {
        const $tabs = $('#editorTabs');
        $tabs.empty();
        currentLayout.tabs.forEach((tab, index) => {
            const isActive = index === activeTabIndex ? 'active' : '';
            const tabId = `tab-${index}`;
            const tabItem = $(`
                <li class="nav-item">
                    <a class="nav-link ${isActive}" href="#" data-index="${index}">
                        <input class="form-control form-control-sm d-inline-block border-0 tab-name" style="width:auto;display:inline;background:transparent;" value="${tab.name}" />
                        <i class="fas fa-pencil-alt ms-2 text-muted" style="opacity:0.5;"></i>
                    </a>
                </li>
            `);
            $tabs.append(tabItem);
        });
        // Tab click handling
        $tabs.find('a.nav-link').on('click', function(e) {
            e.preventDefault();
            activeTabIndex = parseInt($(this).data('index'), 10);
            currentLayout.activeTab = activeTabIndex;
            renderTabs();
            renderDashboard();
        });
        // Inline tab name editing
        $tabs.find('.tab-name').on('change', function() {
            const $link = $(this).closest('a.nav-link');
            const i = parseInt($link.data('index'), 10);
            currentLayout.tabs[i].name = $(this).val();
        });
        // Add remove button to tabs (except if only one tab remains)
        if (currentLayout.tabs.length > 1) {
            $tabs.children().each(function(index) {
                const $li = $(this);
                const removeBtn = $('<button class="btn btn-sm btn-outline-danger ms-2">x</button>');
                removeBtn.on('click', function(e) {
                    e.preventDefault();
                    currentLayout.tabs.splice(index, 1);
                    if (activeTabIndex >= currentLayout.tabs.length) activeTabIndex = currentLayout.tabs.length - 1;
                    currentLayout.activeTab = activeTabIndex;
                    renderTabs();
                    renderDashboard();
                });
                $li.find('a.nav-link').append(removeBtn);
            });
        }
    }

    function renderDashboard() {
        const $grid = $('#dashboardGrid');
        $grid.empty();
        const elements = currentLayout.tabs[activeTabIndex].dashlets || [];
        
        if (elements.length === 0) {
            $grid.append(`
                <div class="dashboard-item col-12" data-size="12" data-name="placeholder">
                    <div class="card border-dashed h-100">
                        <div class="card-body d-flex justify-content-center align-items-center">
                            <span class="text-muted">Drag widgets here to start building your dashboard</span>
                        </div>
                    </div>
                </div>
            `);
        }
        
        elements.forEach(function(item) {
            const name = widgetNameLookup[item.tag] || item.name || item.tag;
            const html = `
              <div class="dashboard-item col-${item.size}" data-size="${item.size}" data-name="${name}" data-tag="${item.tag}">
                <div class="card">
                  <div class="card-body p-2">
                    <div class="d-flex justify-content-between align-items-center">
                      <span>${name}</span>
                      <div>
                        <button class="btn btn-sm btn-outline-secondary decrease-width">-</button>
                        <button class="btn btn-sm btn-outline-secondary increase-width">+</button>
                        <button class="btn btn-sm btn-outline-danger delete-item">x</button>
                      </div>
                    </div>
                  </div>
                </div>
              </div>`;
            $grid.append(html);
        });
    }

    function renderAvailable() {
        const $available = $('#availableList');
        $available.empty();
        // Group by category
        const categories = {};
        availableElements.forEach(item => {
            const cat = item.category || 'Uncategorized';
            if (!categories[cat]) categories[cat] = [];
            categories[cat].push(item);
        });
        // Build accordion groups
        const accordionHTML = `
            <div class="accordion" id="availableAccordion">
                ${Object.entries(categories).map(([category, items], index) => `
                    <div class="accordion-item">
                        <h2 class="accordion-header">
                            <button class="accordion-button ${index === 0 ? '' : 'collapsed'}" 
                                type="button" 
                                data-bs-toggle="collapse" 
                                data-bs-target="#cat-${index}" 
                                aria-expanded="${index === 0 ? 'true' : 'false'}" 
                                aria-controls="cat-${index}">
                                ${category}
                            </button>
                        </h2>
                        <div id="cat-${index}" 
                            class="accordion-collapse collapse ${index === 0 ? 'show' : ''}" 
                            data-bs-parent="#availableAccordion">
                            <div class="accordion-body p-0">
                                <ul class="list-group">
                                    ${items.map(item => `
                                        <li class="list-group-item available-item" 
                                            data-name="${item.name}" 
                                            data-size="${item.size}"
                                            data-tag="${item.tag}">
                                            ${item.name}
                                        </li>
                                    `).join('')}
                                </ul>
                            </div>
                        </div>
                    </div>
                `).join('')}
            </div>
        `;
        $available.html(accordionHTML);
    }

    renderTabs();
    renderDashboard();
    renderAvailable();

    // Handle add tab button
    $('#addTabButton').on('click', function() {
        let newTabName = `Tab ${currentLayout.tabs.length + 1}`;
        currentLayout.tabs.push({
            name: newTabName,
            dashlets: []
        });
        activeTabIndex = currentLayout.tabs.length - 1;
        renderTabs();
        renderDashboard();
    });

    // Initialize SortableJS for the dashboard grid.
    // This allows rearranging within the grid and also accepting items from the available list.
    if (typeof Sortable === 'undefined') {
        console.warn('Dashboard editor: SortableJS is not loaded. Drag/drop will be disabled.');
    } else {
    var dashboardSortable = new Sortable(document.getElementById('dashboardGrid'), {
        animation: 150,
        group: {
            name: 'shared',
            pull: true,
            put: true
        },
        onEnd: function (evt) {
            // Update the order in our data model
            updateDashletOrder();
        },
        onAdd: function (evt) {
            // When an item is dropped into the grid (from the available list),
            // replace it with a fully rendered dashboard item.
            var $item = $(evt.item);
            var name = $item.data('name');
            var size = $item.data('size');
            var tag = $item.data('tag');
            $item.replaceWith(`
        <div class="dashboard-item col-${size}" data-size="${size}" data-name="${name}" data-tag="${tag}">
          <div class="card">
            <div class="card-body p-2">
              <div class="d-flex justify-content-between align-items-center">
                <span>${name}</span>
                <div>
                  <button class="btn btn-sm btn-outline-secondary decrease-width">-</button>
                  <button class="btn btn-sm btn-outline-secondary increase-width">+</button>
                  <button class="btn btn-sm btn-outline-danger delete-item">x</button>
                </div>
              </div>
            </div>
          </div>
        </div>
      `);
            updateDashletOrder();
        }
    });
    }

    // Initialize SortableJS for all available elements lists in the accordion
    document.querySelectorAll('.accordion-body .list-group').forEach(list => {
        if (typeof Sortable === 'undefined') {
            return;
        }
        new Sortable(list, {
            animation: 150,
            group: {
                name: 'shared',
                pull: 'clone',
                put: false
            },
            sort: false
        });
    });

    // Update the dashlet order in our data model based on DOM
    function updateDashletOrder() {
        let newDashlets = [];
        $('#dashboardGrid .dashboard-item').each(function() {
            let $el = $(this);
            if ($el.data('name') !== 'placeholder') {
                // Only store tag and size - name will be looked up from available elements
                newDashlets.push({
                    size: parseInt($el.data('size'), 10),
                    tag: $el.data('tag'),
                });
            }
        });
        currentLayout.tabs[activeTabIndex].dashlets = newDashlets;
    }

    // Handle delete action for dashboard items.
    $('#dashboardGrid').on('click', '.delete-item', function() {
        $(this).closest('.dashboard-item').remove();
        updateDashletOrder();
    });

    // Allow increasing the width of an item (up to 12).
    $('#dashboardGrid').on('click', '.increase-width', function() {
        var $item = $(this).closest('.dashboard-item');
        var size = parseInt($item.data('size'), 10);
        if (size < 12) {
            size++;
            $item.data('size', size);
            // Remove previous col-* class and add the new one.
            $item.removeClass(function(index, className) {
                return (className.match(/(^|\s)col-\S+/g) || []).join(' ');
            }).addClass('col-' + size);
            updateDashletOrder();
        }
    });

    // Allow decreasing the width of an item (minimum 1).
    $('#dashboardGrid').on('click', '.decrease-width', function() {
        var $item = $(this).closest('.dashboard-item');
        var size = parseInt($item.data('size'), 10);
        if (size > 1) {
            size--;
            $item.data('size', size);
            $item.removeClass(function(index, className) {
                return (className.match(/(^|\s)col-\S+/g) || []).join(' ');
            }).addClass('col-' + size);
            updateDashletOrder();
        }
    });

    // Clear current tab
    $('#clearAllButton').on('click', function() {
        if (!confirm('Clear all widgets from this tab?')) return;
        currentLayout.tabs[activeTabIndex].dashlets = [];
        renderDashboard();
    });

    // Restore defaults
    $('#restoreDefaultsButton').on('click', function() {
        if (!confirm('Restore default dashboard layout? This replaces your current layout.')) return;
        // Remove the saved layout for this cookieName and reload page
        localStorage.removeItem(cookieName);
        window.location.reload();
    });

    // Download layout function
    function downloadLayout() {
        const layoutData = {
            version: currentLayout.version || 2,
            tabs: currentLayout.tabs,
            activeTab: activeTabIndex,
            exportedAt: new Date().toISOString(),
            dashboardType: cookieName
        };
        
        const json = JSON.stringify(layoutData, null, 2);
        const blob = new Blob([json], { type: 'application/json' });
        const url = URL.createObjectURL(blob);
        
        const a = document.createElement('a');
        a.href = url;
        a.download = `dashboard-layout-${cookieName}-${Date.now()}.json`;
        document.body.appendChild(a);
        a.click();
        document.body.removeChild(a);
        
        URL.revokeObjectURL(url);
        showAlert('Layout downloaded successfully!', 'success');
    }

    // Validate uploaded layout
    function validateLayout(layout) {
        // Check basic structure
        if (!layout.tabs || !Array.isArray(layout.tabs)) {
            return false;
        }
        
        // Create lookup of valid tags
        const validTags = new Set(availableElements.map(e => e.tag));
        
        // Validate each tab
        for (const tab of layout.tabs) {
            if (!tab.name || !Array.isArray(tab.dashlets)) {
                return false;
            }
            
            // Validate each dashlet
            for (const dashlet of tab.dashlets) {
                if (!dashlet.tag || !dashlet.size) {
                    return false;
                }
                
                // Warn about unknown tags but don't reject
                if (!validTags.has(dashlet.tag)) {
                    console.warn(`Unknown widget tag: ${dashlet.tag}`);
                }
            }
        }
        
        return true;
    }

    // Upload layout function
    function uploadLayout(file) {
        const reader = new FileReader();
        reader.onload = (e) => {
            try {
                const imported = JSON.parse(e.target.result);
                
                // Validate structure
                if (!validateLayout(imported)) {
                    showAlert('Invalid layout file format', 'danger');
                    return;
                }
                
                // Warn if different dashboard type
                if (imported.dashboardType && imported.dashboardType !== cookieName) {
                    if (!confirm(`This layout is from a different dashboard type (${imported.dashboardType}). Continue anyway?`)) {
                        return;
                    }
                }
                
                // Apply the layout
                currentLayout = {
                    version: imported.version || 2,
                    tabs: imported.tabs,
                    activeTab: imported.activeTab || 0
                };
                
                // Re-render
                activeTabIndex = currentLayout.activeTab;
                renderTabs();
                renderDashboard();
                
                showAlert('Layout imported successfully!', 'success');
                
            } catch (err) {
                showAlert('Failed to parse layout file: ' + err.message, 'danger');
            }
        };
        reader.readAsText(file);
    }

    // Handle download button click
    $('#downloadLayoutButton').on('click', function() {
        downloadLayout();
    });

    // Handle upload button click
    $('#uploadLayoutButton').on('click', function() {
        $('#layoutFileInput').click();
    });

    // Handle file selection
    $('#layoutFileInput').on('change', function(e) {
        const file = e.target.files[0];
        if (file) {
            uploadLayout(file);
            // Clear the input so the same file can be selected again
            $(this).val('');
        }
    });

    // Allow increasing the width of an item (up to 12).
    $('#dashboardGrid').on('click', '.increase-width', function() {
        var $item = $(this).closest('.dashboard-item');
        var size = parseInt($item.data('size'), 10);
        if (size < 12) {
            size++;
            $item.data('size', size);
            // Remove previous col-* class and add the new one.
            $item.removeClass(function(index, className) {
                return (className.match(/(^|\s)col-\S+/g) || []).join(' ');
            }).addClass('col-' + size);
            updateDashletOrder();
        }
    });

    // Allow decreasing the width of an item (minimum 1).
    $('#dashboardGrid').on('click', '.decrease-width', function() {
        var $item = $(this).closest('.dashboard-item');
        var size = parseInt($item.data('size'), 10);
        if (size > 1) {
            size--;
            $item.data('size', size);
            $item.removeClass(function(index, className) {
                return (className.match(/(^|\s)col-\S+/g) || []).join(' ');
            }).addClass('col-' + size);
            updateDashletOrder();
        }
    });

    // When the "Done" button is clicked, collect the new layout and call the callback.
    $('#dashboardDone').on('click', function() {
        // Hide and remove the modal.
        $('#dashboardEditorModal').modal('hide').remove();
        // Pass the new layout back to the caller.
        callback({
            version: currentLayout.version || 2,
            tabs: currentLayout.tabs,
            activeTab: activeTabIndex
        });
    });

    // Show the modal using Bootstrap's modal API.
    var modalEl = document.getElementById('dashboardEditorModal');
    var modal = new bootstrap.Modal(modalEl, {
        backdrop: 'static',
        keyboard: false
    });
    modal.show();

    // Clean up when the modal is hidden.
    $('#dashboardEditorModal').on('hidden.bs.modal', function() {
        $(this).remove();
    });
}

