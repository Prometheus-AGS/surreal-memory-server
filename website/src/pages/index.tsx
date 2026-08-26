import type {ReactNode} from 'react';
import Link from '@docusaurus/Link';
import Layout from '@theme/Layout';
import Heading from '@theme/Heading';
import useBaseUrl from '@docusaurus/useBaseUrl';

import styles from './index.module.css';

const readerPaths = [
  {
    label: '01 // Understand',
    title: 'Start with the memory model',
    description:
      'Scopes decide who can read a memory; types decide how it is retrieved. Both are enforced in the query, not by convention.',
    to: '/docs/architecture/memory-model',
  },
  {
    label: '02 // Architect',
    title: 'See the three boundaries',
    description:
      'A published storage trait, a migration-governed schema, and an embedding executor isolated in its own process.',
    to: '/docs/architecture/overview',
  },
  {
    label: '03 // Integrate',
    title: 'Call the MCP surface',
    description:
      'Fifty-nine tools across scoped memory, knowledge graph, TaskStreams, mindmaps, and the optional Memory Palace.',
    to: '/docs/reference/mcp-tools',
  },
  {
    label: '04 // Operate',
    title: 'Deploy and diagnose',
    description:
      'Build features, health and readiness semantics, and the failure modes that have actually occurred in production.',
    to: '/docs/operations/deployment',
  },
] as const;

const layers = [
  ['Memory', 'Scoped, typed, versioned records'],
  ['Graph', 'Entities, relations, Graph-RAG traversal'],
  ['Streams', 'Token-budgeted context for long work'],
  ['Palace', 'A second retrieval space, opt-in'],
] as const;

const surfaces = ['MCP stdio', 'Streamable HTTP', 'SSE', 'REST v1', 'REST v2'] as const;

export default function Home(): ReactNode {
  return (
    <Layout
      title="Surreal Memory Server"
      description="Durable agent memory. Typed storage. One MCP boundary.">
      <main className={styles.page}>
        <section className={styles.hero}>
          <div className={styles.heroCopy}>
            <p className={styles.eyebrow}>Prometheus AGS</p>
            <Heading as="h1" className={styles.title}>
              Durable agent memory
            </Heading>
            <p className={styles.lede}>
              Agents lose everything between turns. This server treats memory as typed,
              scoped, and durable — backed by SurrealDB, retrieved by hybrid search, and
              exposed through one MCP boundary.
            </p>
            <div className={styles.actions}>
              <Link className="button button--primary button--lg" to="/docs/intro">
                Read the docs
              </Link>
              <Link
                className="button button--secondary button--lg"
                to="/docs/design-decisions">
                Why it is built this way
              </Link>
            </div>
            <dl className={styles.runtimeReadout}>
              <span>MCP TOOLS</span>
              <strong>59</strong>
              <span>MIGRATIONS</span>
              <strong>21</strong>
              <span>VECTOR SPACES</span>
              <strong>1536d / 384d</strong>
              <span>RETRIEVAL</span>
              <strong>BM25 + HNSW / RRF</strong>
            </dl>
          </div>
          <div className={styles.brandField}>
            <img
              className={styles.wordmarkLight}
              src={useBaseUrl('/img/brand/sms-wordmark-light.svg')}
              alt="Surreal Memory Server"
            />
            <img
              className={styles.wordmarkDark}
              src={useBaseUrl('/img/brand/sms-wordmark-dark.svg')}
              alt="Surreal Memory Server"
            />
          </div>
        </section>

        <section className={styles.section}>
          <div className={styles.sectionCopy}>
            <Heading as="h2" className={styles.sectionHeading}>
              Four ways in
            </Heading>
          </div>
          <div className={styles.pathGrid}>
            {readerPaths.map((path) => (
              <Link className={styles.pathCard} key={path.label} to={path.to}>
                <p className={styles.eyebrow}>{path.label}</p>
                <Heading as="h3">{path.title}</Heading>
                <p>{path.description}</p>
              </Link>
            ))}
          </div>
        </section>

        <section className={styles.surface}>
          <div className={styles.sectionCopy}>
            <Heading as="h2" className={styles.sectionHeading}>
              What it stores
            </Heading>
          </div>
          <div className={styles.surfaceGrid}>
            {layers.map(([name, detail]) => (
              <div className={styles.boundaryPanel} key={name}>
                <strong>{name}</strong>
                <p>{detail}</p>
              </div>
            ))}
          </div>
        </section>

        <section className={styles.protocolSection}>
          <div className={styles.sectionCopy}>
            <Heading as="h2" className={styles.sectionHeading}>
              Surfaces
            </Heading>
            <p>
              One storage layer behind every transport. The library crate is the
              contract; the binary is a thin shell around it.
            </p>
          </div>
          <ul className={styles.protocolList}>
            {surfaces.map((surface) => (
              <li key={surface}>{surface}</li>
            ))}
          </ul>
        </section>
      </main>
    </Layout>
  );
}
