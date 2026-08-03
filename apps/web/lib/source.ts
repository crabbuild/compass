import { createElement, type ReactNode } from 'react';
import {
  BookOpenIcon,
  BracesIcon,
  CompassIcon,
  FileTextIcon,
  FlaskConicalIcon,
  Layers3Icon,
  MapIcon,
  WrenchIcon,
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

const START_PAGES = ['README.md', 'getting-started.md', 'roadmap.md'] as const;
const COMPASSQL_PAGES = ['concepts/compassql.md', 'COMPASSQL.md', 'COMPASSQL_SUPPORT.md'] as const;

const FOLDER_SECTIONS = [
  { path: 'concepts', name: 'Core concepts', icon: BookOpenIcon },
  { path: 'guides', name: 'Task guides', icon: MapIcon },
  { path: 'cookbook', name: 'Cookbook', icon: FlaskConicalIcon },
  { path: 'reference', name: 'Reference', icon: FileTextIcon },
  { path: 'design', name: 'Design & architecture', icon: Layers3Icon },
  { path: 'implementation', name: 'Implementation', icon: WrenchIcon },
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

function withPageIcon(page: Item, Icon: LucideIcon, name?: string): Item {
  return {
    ...page,
    name: name ?? page.name,
    icon: page.icon ?? sidebarIcon(Icon),
  };
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
    return page ? [withPageIcon(page, BookOpenIcon)] : [];
  });
  const compassqlPages = COMPASSQL_PAGES.flatMap((ref) => {
    const page = getPage(ref);
    return page
      ? [withPageIcon(page, BracesIcon, ref === 'COMPASSQL.md' ? 'Use CompassQL' : undefined)]
      : [];
  });

  const children: Node[] = [
    virtualFolder('start', 'Start here', CompassIcon, startPages),
    virtualFolder('compassql', 'CompassQL', BracesIcon, compassqlPages),
  ];

  for (const section of FOLDER_SECTIONS) {
    const folder = folders.get(section.path);
    if (folder) children.push(withFolderPresentation(folder, section.name, section.icon));
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

  return { ...tree, children };
}

export const source = loader({
  baseUrl: '/docs',
  source: docs.toFumadocsSource(),
  pageTree: {
    transformers: [{ root: organizeDocsTree }],
  },
});
