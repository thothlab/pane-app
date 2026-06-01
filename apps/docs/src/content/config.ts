// Astro content collection registration for Starlight.
// `defineCollection` is the documented entry point as of Starlight 0.30 —
// without this file the `src/content/docs/` tree is invisible.

import { defineCollection } from 'astro:content';
import { docsSchema } from '@astrojs/starlight/schema';

export const collections = {
  docs: defineCollection({ schema: docsSchema() }),
};
