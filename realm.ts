import { createApp, r } from "@yourrealm/sdk";

// Realm sends this with every signed-in request; the app trusts X-Kvitter-User only when it matches.
const proof = r.stableSecret("home-proof");

export default createApp({
  name: "kvitter",
  icon: "ph:bird",
  description: "Short posts, public or private.",

  configSchema: {
    domain: r.home.publicDomain().reason("Serve public posts on their own domain").optional(),
    timezone: r.input.text().title("Time zone")
      .describe("IANA name such as Europe/Oslo, for post times. Empty means UTC."),
  },

  getTrustedHeaders: (user) => ({
    "X-Kvitter-User": user.username,
    "X-Kvitter-Proof": proof,
  }),

  getDesiredState: ({ config }) => ({
    services: {
      web: {
        build: {},
        router: {
          containerPort: 3000,
          publicDomain: config.domain,
          maxRequestSizeMb: 25, // phone photos; the default is 4
          // No CSP here: the app sends its own, with a per-response nonce for the composer.
        },
        env: {
          // Copy public URL, canonical, og:url. Never redirected to.
          PUBLIC_URL: config.domain?.hosts[0] ? `https://${config.domain.hosts[0]}` : "",
          KVITTER_PROOF: proof,
          KVITTER_TZ: config.timezone ?? "",
        },
        volumes: [
          { hostPath: "./data", containerPath: "/data", backup: { kind: "sqlite", file: "kvitter.db" } },
          { hostPath: "./uploads", containerPath: "/uploads", backup: { kind: "file-copy" } },
        ],
        healthcheck: {
          test: ["CMD", "/app/kvitter", "healthcheck"],
          interval: 5, timeout: 3, retries: 12, startPeriod: 2,
        },
      },
    },
  }),
});
