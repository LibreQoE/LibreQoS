# Configuration file for the Sphinx documentation builder.
#
# For the full list of built-in configuration values, see the documentation:
# https://www.sphinx-doc.org/en/master/usage/configuration.html

# -- Project information -----------------------------------------------------
# https://www.sphinx-doc.org/en/master/usage/configuration.html#project-information

import datetime
year = datetime.date.today().year

project = "LibreQoE"
copyright = str(year) + ", LibreQoE, LLC"
author = "LibreQoE, LLC"

# -- General configuration ---------------------------------------------------
# https://www.sphinx-doc.org/en/master/usage/configuration.html#general-configuration

extensions = ["myst_parser"]

myst_enable_extensions = [
    'substitution'
]

templates_path = ["_templates"]
exclude_patterns = ["_build", "Thumbs.db", ".DS_Store"]

# Language configuration
language = "es"

# -- Options for HTML output -------------------------------------------------
# https://www.sphinx-doc.org/en/master/usage/configuration.html#options-for-html-output

html_theme = "sphinx_rtd_theme"
html_static_path = ["_static"]

# Localization settings
locale_dirs = ["locale/"]
gettext_compact = False

# Theme options
html_theme_options = {
    'navigation_depth': 4,
    'collapse_navigation': False,
    'sticky_navigation': True,
    'includehidden': True,
    'titles_only': False
}

def ultimateReplace(app, docname, source):
    result = source[0]
    for key in app.config.ultimate_replacements:
        result = result.replace(key, app.config.ultimate_replacements[key])
    source[0] = result

ultimate_replacements = {
    "{deb_url_v1_5}" : "https://download.libreqos.com/libreqos_1.5-RC2.202510052233-1_amd64.deb"
}

def setup(app):
   app.add_config_value('ultimate_replacements', {}, True)
   app.connect('source-read', ultimateReplace)
