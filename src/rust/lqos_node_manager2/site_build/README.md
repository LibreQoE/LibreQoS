# Building the Website

The node manager site is written in TypeScript, and uses `esbuild` to assemble the output.

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