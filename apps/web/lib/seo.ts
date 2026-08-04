import { githubRepositoryUrl } from '@/lib/github';
import {
  absoluteUrl,
  siteDescription,
  siteImagePath,
  siteName,
  siteUrl,
} from '@/lib/site';

type BreadcrumbItem = {
  name: string;
  path?: string;
};

export function siteJsonLd() {
  const organizationId = `${siteUrl}/#organization`;

  return {
    '@context': 'https://schema.org',
    '@graph': [
      {
        '@type': 'Organization',
        '@id': organizationId,
        name: siteName,
        url: siteUrl,
        logo: absoluteUrl('/brand/compass-mark.svg'),
        description: siteDescription,
        sameAs: [githubRepositoryUrl],
      },
      {
        '@type': 'WebSite',
        '@id': `${siteUrl}/#website`,
        name: siteName,
        url: siteUrl,
        description: siteDescription,
        publisher: { '@id': organizationId },
        inLanguage: 'en-US',
      },
    ],
  };
}

export function softwareApplicationJsonLd() {
  return {
    '@context': 'https://schema.org',
    '@type': 'SoftwareApplication',
    '@id': `${siteUrl}/#software`,
    name: siteName,
    description: siteDescription,
    url: siteUrl,
    image: absoluteUrl(siteImagePath),
    applicationCategory: 'DeveloperApplication',
    operatingSystem: 'macOS, Linux, Windows',
    isAccessibleForFree: true,
    offers: {
      '@type': 'Offer',
      price: '0',
      priceCurrency: 'USD',
    },
    codeRepository: githubRepositoryUrl,
    publisher: { '@id': `${siteUrl}/#organization` },
  };
}

export function breadcrumbJsonLd(items: readonly BreadcrumbItem[]) {
  return {
    '@context': 'https://schema.org',
    '@type': 'BreadcrumbList',
    itemListElement: items.map((item, index) => ({
      '@type': 'ListItem',
      position: index + 1,
      name: item.name,
      ...(item.path ? { item: absoluteUrl(item.path) } : {}),
    })),
  };
}

export function docsArticleJsonLd({
  title,
  description,
  path,
}: {
  title: string;
  description: string;
  path: string;
}) {
  return {
    '@context': 'https://schema.org',
    '@type': 'TechArticle',
    headline: title,
    description,
    url: absoluteUrl(path),
    image: absoluteUrl(siteImagePath),
    mainEntityOfPage: absoluteUrl(path),
    author: { '@id': `${siteUrl}/#organization` },
    publisher: { '@id': `${siteUrl}/#organization` },
    isPartOf: { '@id': `${siteUrl}/#website` },
    inLanguage: 'en-US',
  };
}

export function blogPostingJsonLd({
  title,
  description,
  path,
  author,
  datePublished,
  tags,
}: {
  title: string;
  description: string;
  path: string;
  author: string;
  datePublished: Date;
  tags: readonly string[];
}) {
  return {
    '@context': 'https://schema.org',
    '@type': 'BlogPosting',
    headline: title,
    description,
    url: absoluteUrl(path),
    image: absoluteUrl(siteImagePath),
    mainEntityOfPage: absoluteUrl(path),
    author: {
      '@type': 'Organization',
      name: author,
      url: siteUrl,
    },
    publisher: { '@id': `${siteUrl}/#organization` },
    datePublished: datePublished.toISOString(),
    articleSection: tags[0],
    keywords: tags,
    inLanguage: 'en-US',
  };
}
