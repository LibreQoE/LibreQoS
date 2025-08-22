# Configuration file for the Sphinx documentation builder.
#
# For the full list of built-in configuration values, see the documentation:
# https://www.sphinx-doc.org/en/master/usage/configuration.html

# -- Project information -----------------------------------------------------
# https://www.sphinx-doc.org/en/master/usage/configuration.html#project-information

project = "LibreQoE"
copyright = "2024, LibreQoE, LLC"
author = "Zach Biles"

# -- General configuration ---------------------------------------------------
# https://www.sphinx-doc.org/en/master/usage/configuration.html#general-configuration

extensions = ["myst_parser"]

templates_path = ["_templates"]
exclude_patterns = ["_build", "Thumbs.db", ".DS_Store"]

rst_prolog = """
.. |deb_file_zip_url| replace:: https://libreqos.io/wp-content/uploads/2025/08/libreqos_1.5-RC1.202508211229-1_amd64.zip
"""
rst_prolog = """
.. |deb_file_zip_name| replace:: libreqos_1.5-RC1.202508211229-1_amd64.zip
"""
rst_prolog = """
.. |deb_file_name| replace:: libreqos_1.5-RC1.202508211229-1_amd64.deb
"""

# -- Options for HTML output -------------------------------------------------
# https://www.sphinx-doc.org/en/master/usage/configuration.html#options-for-html-output

html_theme = "sphinx_rtd_theme"
html_static_path = ["_static"]
