# LibreQoS Setup

This is the setup tool for LibreQoS, designed to be run automatically as part of the `dpkg` installation process. Its primary goal is to help users quickly establish a minimal working configuration for LibreQoS, ensuring that the system is ready for operation with minimal manual intervention.

## Purpose

The LibreQoS setup tool guides users through the essential configuration steps required to get LibreQoS up and running. It simplifies the initial setup process by providing a guided interface and automating the creation of necessary configuration files.

## What the Setup Tool Configures

The tool assists in configuring the following components:

- **Bridge Mode**: Select between Linux, XDP, or Single bridge modes to match your deployment scenario.
- **Network Interfaces**: Choose and configure the network interfaces that LibreQoS will manage.
- **Bandwidth Settings**: Set bandwidth limits and parameters for your network environment.
- **IP Ranges**: Define the IP address ranges that LibreQoS should monitor and shape.
- **Web Users**: Create and manage web user accounts for accessing the LibreQoS web interface.
- **Configuration Files**: Automatically generates and updates key configuration files, including:
  - `lqos.conf`
  - `network.json`
  - `ShapedDevices.csv`

## User Interface

The setup tool provides a TUI (Text User Interface) built with the [Cursive](https://github.com/gyscos/cursive) library. This interface allows users to navigate configuration options interactively in the terminal, making the setup process straightforward and user-friendly.

## Integration with LibreQoS

By running this tool during installation, LibreQoS ensures that all critical settings are defined and that the system is ready for immediate use. The setup tool is an integral part of the overall LibreQoS system, streamlining deployment and reducing the potential for misconfiguration.
