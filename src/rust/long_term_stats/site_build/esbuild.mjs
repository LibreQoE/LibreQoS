#!/usr/bin/env node
import * as esbuild from 'esbuild'

await esbuild.build({
  entryPoints: ['src/app.ts', 'src/style.css'],
  bundle: true,
  minify: true,
  sourcemap: true,
//  target: ['chrome58', 'firefox57', 'safari11', 'edge16'],
  outdir: 'output/',
  loader: { '.html': 'text'},
  format: 'esm',
})
