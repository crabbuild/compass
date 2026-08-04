import { createElement, type ReactNode } from 'react';
import {
  BookOpenIcon,
  BracesIcon,
  CompassIcon,
  FileTextIcon,
  FlaskConicalIcon,
  MapIcon,
  type LucideIcon,
} from 'lucide-react';
import type { Folder, Item, Node, Root } from 'fumadocs-core/page-tree';
import { docs } from 'collections/server';
import { loader } from 'fumadocs-core/source';

const sidebarIcon = (Icon: LucideIcon): ReactNode =>
  createElement(Icon, {
    'aria-hidden': true,
    className: 'docs-sidebar-icon',
  });

const START_PAGES = ['getting-started.md', 'README.md'] as const;
const COMPASSQL_PAGES = ['concepts/compassql.md', 'COMPASSQL.md', 'COMPASSQL_SUPPORT.md'] as const;
const COOKBOOK_PAGES = [
  'cookbook/README.md',
  'cookbook/architecture-discovery.md',
  'cookbook/ci-and-automation.md',
  'cookbook/impact-analysis.md',
  'cookbook/troubleshooting.md',
] as const;

const FOLDER_SECTIONS = [
  { path: 'concepts', name: 'Core concepts', icon: BookOpenIcon },
  { path: 'guides', name: 'Task guides', icon: MapIcon },
  { path: 'cookbook', name: 'Cookbook', icon: FlaskConicalIcon },
  { path: 'reference', name: 'Reference', icon: FileTextIcon },
] as const;

function collectPages(nodes: Node[], pages = new Map<string, Item>()): Map<string, Item> {
  for (const node of nodes) {
    if (node.type === 'page' && node.$ref) pages.set(node.$ref, node);
    if (node.type === 'folder') {
      if (node.index?.$ref) pages.set(node.index.$ref, node.index);
      collectPages(node.children, pages);
    }
  }
  return pages;
}

function removePages(
  nodes: Node[],
  refs: Set<string>,
  extracted: Map<string, Item>,
): Node[] {
  const output: Node[] = [];

  for (const node of nodes) {
    if (node.type === 'page') {
      if (node.$ref && refs.has(node.$ref)) extracted.set(node.$ref, node);
      else output.push(node);
      continue;
    }

    if (node.type !== 'folder') {
      output.push(node);
      continue;
    }

    let index = node.index;
    if (index?.$ref && refs.has(index.$ref)) {
      extracted.set(index.$ref, index);
      index = undefined;
    }

    output.push({
      ...node,
      index,
      children: removePages(node.children, refs, extracted),
    });
  }

  return output;
}

function withPageName(page: Item, name: string): Item {
  return {
    ...page,
    name,
  };
}

function withCookbookPageName(page: Item): Item {
  if (page.$ref === 'cookbook/README.md') return withPageName(page, 'Overview');
  if (typeof page.name !== 'string') return page;

  const name = page.name.replace(/^Cookbook:\s*/i, '');
  const capitalizedName = name ? `${name[0].toUpperCase()}${name.slice(1)}` : name;
  return capitalizedName === page.name ? page : withPageName(page, capitalizedName);
}

function keepOnlyTopLevelIcons(nodes: Node[], depth = 0): Node[] {
  return nodes.map((node) => {
    if (node.type === 'page') return { ...node, icon: undefined };
    if (node.type !== 'folder') return node;

    return {
      ...node,
      icon: depth === 0 ? node.icon : undefined,
      index: node.index ? { ...node.index, icon: undefined } : undefined,
      children: keepOnlyTopLevelIcons(node.children, depth + 1),
    };
  });
}

function withFolderPresentation(
  folder: Folder,
  name: string,
  Icon: LucideIcon,
): Folder {
  return {
    ...folder,
    name,
    icon: folder.icon ?? sidebarIcon(Icon),
    collapsible: folder.collapsible ?? true,
  };
}

function withCookbookPresentation(folder: Folder): Folder {
  const presented = withFolderPresentation(folder, 'Cookbook', FlaskConicalIcon);
  const orderedRefs = new Set<string>(COOKBOOK_PAGES);
  const pagesByRef = new Map<string, Item>();
  const nonPageChildren: Node[] = [];

  for (const node of presented.children) {
    if (node.type === 'page' && node.$ref) pagesByRef.set(node.$ref, node);
    else nonPageChildren.push(node);
  }

  const orderedPages = COOKBOOK_PAGES.flatMap((ref) => {
    const page = pagesByRef.get(ref);
    return page ? [withCookbookPageName(page)] : [];
  });
  const remainingPages = [...pagesByRef.entries()]
    .filter(([ref]) => !orderedRefs.has(ref))
    .map(([, page]) => withCookbookPageName(page));

  return {
    ...presented,
    index: presented.index ? withCookbookPageName(presented.index) : undefined,
    children: [...orderedPages, ...remainingPages, ...nonPageChildren],
  };
}

function virtualFolder(
  id: string,
  name: string,
  Icon: LucideIcon,
  children: Item[],
): Folder {
  return {
    $id: `compass-docs-${id}`,
    type: 'folder',
    name,
    icon: sidebarIcon(Icon),
    defaultOpen: true,
    collapsible: true,
    children,
  };
}

function organizeDocsTree(tree: Root): Root {
  const pages = collectPages(tree.children);
  const compassqlRefs = new Set<string>([...START_PAGES, ...COMPASSQL_PAGES]);
  const extracted = new Map<string, Item>();
  const withoutGroupedPages = removePages(tree.children, compassqlRefs, extracted);
  const folders = new Map<string, Folder>();
  const remaining = withoutGroupedPages.filter((node) => {
    if (node.type !== 'folder' || !node.$ref?.folder) return true;
    const section = FOLDER_SECTIONS.find((item) => item.path === node.$ref?.folder);
    if (!section) return true;
    folders.set(section.path, node);
    return false;
  });

  const getPage = (ref: string): Item | undefined => extracted.get(ref) ?? pages.get(ref);
  const startPages = START_PAGES.flatMap((ref) => {
    const page = getPage(ref);
    return page
      ? [ref === 'README.md' ? withPageName(page, 'Documentation map') : page]
      : [];
  });
  const compassqlPages = COMPASSQL_PAGES.flatMap((ref) => {
    const page = getPage(ref);
    return page
      ? [ref === 'COMPASSQL.md' ? withPageName(page, 'Use CompassQL') : page]
      : [];
  });

  const children: Node[] = [virtualFolder('start', 'Start here', CompassIcon, startPages)];

  for (const section of FOLDER_SECTIONS) {
    const folder = folders.get(section.path);
    if (folder) {
      children.push(section.path === 'cookbook'
        ? withCookbookPresentation(folder)
        : withFolderPresentation(folder, section.name, section.icon));
    }
    if (section.path === 'cookbook') children.push(virtualFolder('compassql', 'CompassQL', BracesIcon, compassqlPages));
  }

  if (remaining.length > 0) {
    children.push({
      $id: 'compass-docs-more',
      type: 'folder',
      name: 'More documentation',
      icon: sidebarIcon(FileTextIcon),
      collapsible: true,
      children: remaining,
    });
  }

  return { ...tree, children: keepOnlyTopLevelIcons(children) };
}

export const source = loader({
  baseUrl: '/docs',
  source: docs.toFumadocsSource(),
  slugs: (file) => {
    if (file.path === 'README.md') return ['docmap'];
    if (file.path === 'cookbook/README.md') return ['cookbook', 'overview'];
    return undefined;
  },
  pageTree: {
    transformers: [{ root: organizeDocsTree }],
  },
});
