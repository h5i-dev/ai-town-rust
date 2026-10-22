import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

const api = process.env.AI_TOWN_API_URL ?? 'http://127.0.0.1:8080';

export default defineConfig({
  base: '/ai-town/',
  plugins: [react()],
  server: {
    host: '127.0.0.1',
    port: 5173,
    proxy: {
      '/rpc': api,
      '/ws': { target: api, ws: true },
      '/map': api,
      '/snapshot': api,
      '/session': api,
      '/healthz': api,
    },
  },
});
