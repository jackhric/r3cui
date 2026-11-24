import sitemap from '@astrojs/sitemap';
import starlight from '@astrojs/starlight';
import tailwindcss from '@tailwindcss/vite';
import { defineConfig } from 'astro/config';
import { statisticsHead } from './src/statistics.mjs';

const repo = 'https://github.com/jackhric/r3cui';
const ogImage = 'https://jackhric.github.io/r3cui/og.png';

const building = process.argv.includes('build');

export default defineConfig({
  site: 'https://jackhric.github.io',
  base: '/r3cui',
  integrations: [
    // One entry per page: the trailing-slash form only (the bare /r3cui duplicated the home page).
    sitemap({ filter: page => page.endsWith('/') && !page.includes('/og-card/') }),
    starlight({
      title: 'r3cui',
      description: 'A simplified Unfolded Circle Remote 3 interface for your home theater.',
      components: { SiteTitle: './src/components/docs/SiteTitle.astro' },
      favicon: '/favicon.svg',
      social: [{ icon: 'github', label: 'GitHub', href: repo }],
      editLink: { baseUrl: `${repo}/edit/dev/site/` },
      lastUpdated: true,
      customCss: ['./src/styles/docs.css'],
      head: [
        ...(building ? statisticsHead : []),
        { tag: 'meta', attrs: { property: 'og:image', content: ogImage } },
        { tag: 'meta', attrs: { property: 'og:image:width', content: '1200' } },
        { tag: 'meta', attrs: { property: 'og:image:height', content: '630' } },
        { tag: 'meta', attrs: { name: 'twitter:card', content: 'summary_large_image' } },
        { tag: 'meta', attrs: { name: 'twitter:image', content: ogImage } },
        { tag: 'meta', attrs: { name: 'twitter:image:alt', content: 'r3cui: a simplified custom frontend for the Unfolded Circle Remote 3, shown on three remotes' } },
        { tag: 'meta', attrs: { property: 'og:image:type', content: 'image/png' } },
        { tag: 'link', attrs: { rel: 'manifest', href: '/r3cui/manifest.webmanifest' } },
        { tag: 'link', attrs: { rel: 'apple-touch-icon', href: '/r3cui/apple-touch-icon.png' } },
        { tag: 'link', attrs: { rel: 'alternate', type: 'text/plain', title: 'r3cui for AI assistants', href: '/r3cui/llms.txt' } },
      ],
      sidebar: [{ label: 'Start here', items: ['docs', 'docs/getting-started', 'docs/faq'] }],
    }),
  ],
  vite: {
    plugins: [tailwindcss()],
    server: { fs: { allow: ['..'] } },
  },
});
