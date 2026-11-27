import type { APIRoute } from 'astro';
import { getCollection } from 'astro:content';

// llms-full.txt: every docs page's source in one plain-text file, for AI assistants that want the whole thing.
export const GET: APIRoute = async ({ site }) => {
  const base = new URL(import.meta.env.BASE_URL.replace(/\/?$/, '/'), site);
  const docs = (await getCollection('docs')).sort((a, b) => a.id.localeCompare(b.id));
  const parts = docs.map(d => `# ${d.data.title}\n\nURL: ${new URL(`${d.id.replace(/\/index$/, '')}/`, base)}\n${d.data.description ? `\n${d.data.description}\n` : ''}\n${(d.body ?? '').trim()}\n`);
  return new Response(['# r3cui documentation', '', ...parts].join('\n'), { headers: { 'Content-Type': 'text/plain; charset=utf-8' } });
};
