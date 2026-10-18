import { readFileSync } from 'node:fs';
import { defineConfig, type Plugin } from 'vite';
import react from '@vitejs/plugin-react';

// In production the platform container serves /runtime-config.json; in `npm run dev` this does.
const devRuntimeConfig: Plugin = {
  name: 'picus-dev-runtime-config',
  configureServer(server) {
    server.middlewares.use('/runtime-config.json', (_req, res) => {
      res.setHeader('content-type', 'application/json');
      res.end(readFileSync('runtime-config.dev.json'));
    });
  },
};

export default defineConfig({
  plugins: [react(), devRuntimeConfig],
});
