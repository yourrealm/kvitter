import { defineConfig, devices } from "@playwright/test";

const PORT = 3100;

export default defineConfig({
  testDir: "e2e",
  // One server and one database for the whole run.
  workers: 1,
  fullyParallel: false,
  forbidOnly: !!process.env.CI,
  reporter: [["list"]],
  use: {
    baseURL: `http://localhost:${PORT}`,
    trace: "retain-on-failure",
  },
  projects: [
    { name: "desktop", use: { ...devices["Desktop Chrome"] } },
    { name: "phone", use: { ...devices["Pixel 7"] } },
  ],
  webServer: {
    command: "rm -rf e2e/.data && pnpm build:web && cargo run --quiet -- serve",
    url: `http://localhost:${PORT}/healthz`,
    timeout: 300_000,
    reuseExistingServer: false,
    env: {
      PORT: String(PORT),
      KVITTER_PROOF: "e2e-proof",
      PUBLIC_URL: "https://kvitter.example",
      DATA_DIR: "e2e/.data",
      UPLOADS_DIR: "e2e/.data/uploads",
    },
  },
});
