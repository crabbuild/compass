import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

const pages = import.meta.glob('./src/**/*.tsx', { eager: true, query: '?raw' });

export default defineConfig({
  plugins: [react()],
  resolve: { alias: { '~': './src' } },
});
