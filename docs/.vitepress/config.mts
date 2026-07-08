import { defineConfig } from "vitepress";

// Deployed to GitHub Pages at https://irongollem.github.io/stl-pack/
// by .github/workflows/docs.yml on every push to main touching docs/.
export default defineConfig({
  title: "Plinth",
  description:
    "An opinionated desktop tool for cataloging, rendering, and sharing 3D-printable model libraries",
  base: "/stl-pack/",
  cleanUrls: true,
  lastUpdated: true,
  themeConfig: {
    nav: [
      { text: "Guide", link: "/guide/getting-started" },
      { text: "For creators", link: "/CREATORS" },
      { text: "Reference", link: "/3PK" },
      {
        text: "Download",
        link: "https://github.com/irongollem/stl-pack/releases",
      },
    ],
    sidebar: [
      {
        text: "Guide",
        items: [
          { text: "Installation", link: "/INSTALL" },
          { text: "Getting started", link: "/guide/getting-started" },
          { text: "Sharing releases", link: "/guide/sharing" },
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
      { icon: "github", link: "https://github.com/irongollem/stl-pack" },
    ],
    search: { provider: "local" },
    outline: [2, 3],
  },
});
