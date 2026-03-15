import type { CapacitorConfig } from '@capacitor/cli';

const serverUrl =
  // Override per sync/run:
  // THRONGTERM_IOS_SERVER_URL=http://<YOUR_TAILSCALE_IP>:5173 npx cap sync ios
  process.env.THRONGTERM_IOS_SERVER_URL?.trim() ||
  'http://100.101.123.63:3210/';

const config: CapacitorConfig = {
  appId: 'com.throngterm.app',
  appName: 'throngterm',
  webDir: '../dist',
  server: {
    url: serverUrl,
    cleartext: serverUrl.startsWith('http://'),
    errorPath: 'mobile-error.html',
  },
};

export default config;
