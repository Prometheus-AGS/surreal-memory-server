import type {SidebarsConfig} from '@docusaurus/plugin-content-docs';

const sidebars: SidebarsConfig = {
  docsSidebar: [
    'intro',
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
      items: ['reference/mcp-tools', 'reference/configuration', 'reference/migrations'],
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
