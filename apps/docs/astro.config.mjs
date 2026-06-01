// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

export default defineConfig({
  // The deployed home is pane.thothlab.tech with docs mounted under
  // /docs. `base` prefixes every generated asset / link so CSS, JS,
  // sitemap and internal navigation resolve correctly once served from
  // the sub-path. `site` is used for canonical / sitemap absolute URLs.
  site: 'https://pane.thothlab.tech',
  base: '/docs',
  integrations: [
    starlight({
      title: 'Pane',
      description:
        'Network debugger for mobile apps. MITM HTTPS proxy, one-command iOS / Android setup, response stubs and patches.',
      logo: { src: './public/logo-mark.png', replacesTitle: false },
      social: {
        github: 'https://github.com/thothlab/pane-app',
      },
      editLink: {
        baseUrl: 'https://github.com/thothlab/pane-app/edit/main/apps/docs/',
      },
      lastUpdated: true,
      tableOfContents: { minHeadingLevel: 2, maxHeadingLevel: 3 },
      customCss: ['./src/styles/pane.css'],
      components: {
        ThemeSelect: './src/components/ThemeToggle.astro',
      },
      sidebar: [
        {
          label: 'Start here',
          items: [
            { label: 'What is Pane', link: '/' },
            { label: 'Getting started', link: '/getting-started/' },
          ],
        },
        {
          label: 'Features',
          items: [
            { label: 'Response stubs', link: '/rules/' },
            { label: 'Filtering captures', link: '/filtering/' },
          ],
        },
        {
          label: 'Reference',
          items: [
            { label: 'Release process', link: '/reference/releases/' },
          ],
        },
      ],
    }),
  ],
});
