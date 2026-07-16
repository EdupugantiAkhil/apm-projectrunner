import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// https://vite.dev/config/
export default defineConfig({
  // The daemon serves this build beneath `/gui/`, not at the web-server root.
  base: './',
  plugins: [react()],
})
