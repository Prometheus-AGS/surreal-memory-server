import {themes as prismThemes} from 'prism-react-renderer';
import type {Config} from '@docusaurus/types';
import type * as Preset from '@docusaurus/preset-classic';

const config: Config = {
  title: 'Surreal Memory Server',
  tagline: 'Durable agent memory. Typed storage. One MCP boundary.',
  favicon: 'img/brand/sms-favicon.svg',

  future: {
    v4: true,
  },

  url: 'https://prometheus-ags.github.io',
  baseUrl: '/surreal-memory-server/',

  organizationName: 'Prometheus-AGS',
  projectName: 'surreal-memory-server',
  trailingSlash: false,

  onBrokenLinks: 'throw',

  markdown: {
    mermaid: true,
    hooks: {
      onBrokenMarkdownLinks: 'throw',
    },
  },

  themes: [
    '@docusaurus/theme-mermaid',
    [
      '@easyops-cn/docusaurus-search-local',
      {
        hashed: 'filename',
        language: ['en'],
        indexDocs: true,
        indexBlog: false,
        indexPages: true,
        docsRouteBasePath: '/docs',
        searchResultLimits: 8,
        searchBarShortcutKeymap: 'mod+k',
      },
    ],
  ],

  i18n: {
    defaultLocale: 'en',
    locales: ['en'],
  },

  presets: [
    [
      'classic',
      {
        docs: {
          routeBasePath: '/docs',
          sidebarPath: './sidebars.ts',
          editUrl:
            'https://github.com/Prometheus-AGS/surreal-memory-server/tree/main/website/',
        },
        blog: false,
        theme: {
          customCss: './src/css/custom.css',
        },
      } satisfies Preset.Options,
    ],
  ],

  themeConfig: {
    image: 'img/brand/sms-social-card.svg',
    colorMode: {
      defaultMode: 'dark',
      respectPrefersColorScheme: true,
    },
    navbar: {
      title: 'Surreal Memory Server',
      logo: {
        alt: 'Surreal Memory Server',
        src: 'img/brand/sms-mark-light.svg',
        srcDark: 'img/brand/sms-mark-dark.svg',
      },
      items: [
        {type: 'docSidebar', sidebarId: 'docsSidebar', position: 'left', label: 'Docs'},
        {
          href: 'https://github.com/Prometheus-AGS/surreal-memory-server',
          label: 'GitHub',
          position: 'right',
        },
      ],
    },
    footer: {
      style: 'dark',
      links: [
        {
          title: 'Docs',
          items: [
            {label: 'Introduction', to: '/docs/intro'},
            {label: 'Architecture', to: '/docs/architecture/overview'},
            {label: 'Operations', to: '/docs/operations/deployment'},
          ],
        },
        {
          title: 'Prometheus',
          items: [
            {
              label: 'Universal Agent Runtime',
              href: 'https://github.com/Prometheus-AGS/universal-agent-runtime',
            },
          ],
        },
      ],
      copyright: `Copyright © ${new Date().getFullYear()} Prometheus AGS.`,
    },
    prism: {
      theme: prismThemes.github,
      darkTheme: prismThemes.dracula,
      additionalLanguages: ['rust', 'bash', 'json', 'toml'],
    },
  } satisfies Preset.ThemeConfig,
};

export default config;
