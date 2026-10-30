import { defineConfig } from 'vite';
import vue from '@vitejs/plugin-vue';

// Relative base so the build works at https://<user>.github.io/<repo>/ and
// anywhere else without knowing the path. Routing uses the URL hash.
export default defineConfig({
  base: './',
  plugins: [vue()],
  worker: { format: 'es' },
});
