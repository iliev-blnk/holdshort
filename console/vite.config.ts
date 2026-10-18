import { readFileSync, readdirSync } from 'node:fs';
import { extname, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import type { Plugin } from 'vite';
import { defineConfig } from 'vitest/config';
import { renderWriteup } from './src/writeup/render.ts';

const root = fileURLToPath(new URL('.', import.meta.url));
const TYPES: Record<string, string> = { '.png': 'image/png', '.jpg': 'image/jpeg', '.svg': 'image/svg+xml' };

/** Render docs/WRITEUP.md into writeup.html and ship its figures under writeup-figures/. */
export function writeupPlugin(mdPath: string, figuresDir: string): Plugin {
  return {
    name: 'holdshort-writeup',
    transformIndexHtml: {
      // After Vite's own HTML processing, so the figure paths stay as written.
      order: 'post',
      handler(html, ctx) {
        if (!ctx.filename.endsWith('writeup.html')) return html;
        return html.replace('<!--writeup-->', renderWriteup(readFileSync(mdPath, 'utf8')));
      },
    },
    configureServer(server) {
      server.watcher.add(mdPath);
      server.watcher.on('change', (f) => {
        if (resolve(f) === mdPath) server.ws.send({ type: 'full-reload' });
      });
      server.middlewares.use('/writeup-figures', (req, res, next) => {
        const f = resolve(figuresDir, decodeURIComponent((req.url ?? '').split('?')[0].replace(/^\//, '')));
        if (!f.startsWith(figuresDir + sep)) return next();
        try {
          const body = readFileSync(f);
          res.setHeader('Content-Type', TYPES[extname(f)] ?? 'application/octet-stream');
          res.end(body);
        } catch {
          next();
        }
      });
    },
    generateBundle() {
      for (const name of readdirSync(figuresDir)) {
        this.emitFile({ type: 'asset', fileName: `writeup-figures/${name}`, source: readFileSync(resolve(figuresDir, name)) });
      }
    },
  };
}

export default defineConfig({
  base: './',
  plugins: [writeupPlugin(resolve(root, '../docs/WRITEUP.md'), resolve(root, '../docs/writeup-figures'))],
  build: {
    target: 'es2022',
    chunkSizeWarningLimit: 2000,
    rollupOptions: { input: { main: resolve(root, 'index.html'), writeup: resolve(root, 'writeup.html') } },
  },
  test: { environment: 'node', include: ['test/**/*.test.ts'] },
});
