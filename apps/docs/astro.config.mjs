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
        'Сетевой отладчик для мобильных приложений. MITM HTTPS-прокси, настройка устройств одной кнопкой, подмена и патчинг ответов.',
      defaultLocale: 'root',
      locales: {
        root: { label: 'Русский', lang: 'ru' },
        en: { label: 'English', lang: 'en' },
      },
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
          label: 'С чего начать',
          translations: { en: 'Start here' },
          items: [
            {
              label: 'Что такое Pane',
              translations: { en: 'What is Pane' },
              link: '/',
            },
            {
              label: 'Начало работы',
              translations: { en: 'Getting started' },
              link: '/getting-started/',
            },
          ],
        },
        {
          label: 'Возможности',
          translations: { en: 'Features' },
          items: [
            {
              label: 'Подмена ответов',
              translations: { en: 'Response stubs' },
              link: '/rules/',
            },
            {
              label: 'Фильтрация captures',
              translations: { en: 'Filtering captures' },
              link: '/filtering/',
            },
          ],
        },
        {
          label: 'Справочник',
          translations: { en: 'Reference' },
          items: [
            {
              label: 'Релизный процесс',
              translations: { en: 'Release process' },
              link: '/reference/releases/',
            },
          ],
        },
      ],
    }),
  ],
});
