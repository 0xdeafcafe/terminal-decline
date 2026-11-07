import { defineConfig } from 'astro/config';
import react from '@astrojs/react';

export default defineConfig({
  site: 'https://decline.forbes.red',
  output: 'static',
  integrations: [react()],
  devToolbar: { enabled: false },
});
