import { defineConfig } from "vitepress";

// Deployed to GitHub Pages at https://irongollem.github.io/plinth/
// by .github/workflows/docs.yml on every push to main touching docs/.
export default defineConfig({
  title: "Plinth",
  description:
    "An opinionated desktop tool for cataloging, rendering, and distributing 3D-printable model libraries",
  base: "/plinth/",
  cleanUrls: true,
  lastUpdated: true,
  themeConfig: {
    nav: [
      { text: "Guide", link: "/guide/getting-started" },
      { text: "For creators", link: "/CREATORS" },
      { text: "Reference", link: "/3PK" },
      {
        text: "Download",
        link: "https://github.com/irongollem/plinth/releases",
      },
    ],
    sidebar: [
      {
        text: "Guide",
        items: [
          { text: "Installation", link: "/INSTALL" },
          { text: "Getting started", link: "/guide/getting-started" },
          { text: "Distributing & moving", link: "/guide/distributing" },
        ],
      },
      {
        text: "For creators",
        items: [{ text: "Releasing with Plinth", link: "/CREATORS" }],
      },
      {
        text: "Reference",
        items: [
          { text: ".3pk format specification", link: "/3PK" },
          { text: "Catalog internals", link: "/CATALOG" },
        ],
      },
    ],
    socialLinks: [
      { icon: "github", link: "https://github.com/irongollem/plinth" },
    ],
    search: { provider: "local" },
    outline: [2, 3],
  },
});
