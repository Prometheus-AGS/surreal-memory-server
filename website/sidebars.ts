import type {SidebarsConfig} from '@docusaurus/plugin-content-docs';

const sidebars: SidebarsConfig = {
  docsSidebar: [
    'intro',
    'whats-new',
    {
      type: 'category',
      label: 'Guides',
      items: [
        'guides/quickstart',
        'guides/authentication',
        'guides/durable-operations',
        'guides/memory-expiry',
        'guides/compression',
      ],
    },
    {
      type: 'category',
      label: 'Architecture',
      items: [
        'architecture/overview',
        'architecture/storage',
        'architecture/embedding-executor',
        'architecture/memory-model',
      ],
    },
    {
      type: 'category',
      label: 'Reference',
      items: [
        'reference/rest-api',
        'reference/mcp-tools',
        'reference/cli',
        'reference/configuration',
        'reference/migrations',
      ],
    },
    {
      type: 'category',
      label: 'Operations',
      items: ['operations/deployment', 'operations/troubleshooting'],
    },
    'design-decisions',
  ],
};

export default sidebars;
