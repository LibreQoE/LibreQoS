# Site Build

This folder compiles and packages the website used by `lts_node`. It
needs to be compiled and made available to the `lts_node` process.

Steps: TBA

## Requirements

To run the build (as opposed to shipping pre-built files), you need to
install `esbuild` and `npm` (ugh). You can do this with:

```bash
(change directory to site_build folder)
sudo apt-get install npm
npm install
````

You can run the build manually by running `./esbuild.sh` in this
directory.