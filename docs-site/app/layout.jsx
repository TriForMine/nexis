import { Footer, Layout, Navbar } from 'nextra-theme-docs'
import { Banner, Head } from 'nextra/components'
import { getPageMap } from 'nextra/page-map'
import 'nextra-theme-docs/style.css'

export const metadata = {
  title: {
    default: 'NEXIS',
    template: '%s | NEXIS Docs'
  },
  description:
    'Open-source, engine-agnostic multiplayer backend with Rust data plane and hosted-ready control plane.'
}

const banner = (
  <Banner storageKey="nexis-beta-banner">
    NEXIS is in beta. APIs are stable for MVP, but some surfaces may evolve.
  </Banner>
)

const navbar = <Navbar logo={<b>NEXIS</b>} />

const footer = (
  <Footer>
    <span>Apache-2.0 © {new Date().getFullYear()} TriForMine.</span>
  </Footer>
)

export default async function RootLayout({ children }) {
  return (
    <html lang="en" dir="ltr" suppressHydrationWarning>
      <Head />
      <body>
        <Layout
          banner={banner}
          navbar={navbar}
          pageMap={await getPageMap()}
          docsRepositoryBase="https://github.com/TriForMine/nexis/tree/main/docs-site/content"
          footer={footer}
          sidebar={{
            defaultMenuCollapseLevel: 1,
            toggleButton: true
          }}
          toc={{
            backToTop: true
          }}
        >
          {children}
        </Layout>
      </body>
    </html>
  )
}
