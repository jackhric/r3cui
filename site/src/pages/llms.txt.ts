import type { APIRoute } from 'astro';
import { getCollection } from 'astro:content';

// llms.txt: a short plain-text map of the site for AI assistants and answer engines (llmstxt.org).
export const GET: APIRoute = async ({ site }) => {
  const base = new URL(import.meta.env.BASE_URL.replace(/\/?$/, '/'), site);
  const docs = (await getCollection('docs')).sort((a, b) => a.id.localeCompare(b.id));
  const lines = [
    '# r3cui',
    '',
    '> r3cui (Remote 3 Custom UI) is a free, open-source (GPL-3.0) replacement for the touchscreen UI on the Unfolded Circle Remote 3. It is a simplified, single-purpose home theater remote: Kodi now playing and library, receiver inputs through Home Assistant, global volume, and button backlighting that follows what is on screen. It is beta software; installing it voids the remote\'s warranty.',
    '',
    '## Docs',
    '',
    ...docs.map(d => `- [${d.data.title}](${new URL(`${d.id.replace(/\/index$/, '')}/`, base)}): ${d.data.description ?? ''}`),
    '',
    '## Project',
    '',
    '- [Source code on GitHub](https://github.com/jackhric/r3cui): Rust, Slint; README with install steps',
    '- [Releases](https://github.com/jackhric/r3cui/releases): installable r3cui-<version>.tar.gz and install scripts',
    `- [Full text](${new URL('llms-full.txt', base)}): every docs page in one file`,
    '',
  ];
  return new Response(lines.join('\n'), { headers: { 'Content-Type': 'text/plain; charset=utf-8' } });
};
