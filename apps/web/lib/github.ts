export const githubRepository = 'crabbuild/compass';
export const githubRepositoryUrl = `https://github.com/${githubRepository}`;

type GitHubRepositoryResponse = {
  stargazers_count?: unknown;
};

export async function getGitHubStarCount(): Promise<number | null> {
  try {
    const response = await fetch(`https://api.github.com/repos/${githubRepository}`, {
      headers: {
        Accept: 'application/vnd.github+json',
        'X-GitHub-Api-Version': '2022-11-28',
      },
      next: { revalidate: 3600 },
    });

    if (!response.ok) return null;

    const repository = (await response.json()) as GitHubRepositoryResponse;
    return typeof repository.stargazers_count === 'number' ? repository.stargazers_count : null;
  } catch {
    return null;
  }
}

export function formatGitHubStarCount(count: number): string {
  return new Intl.NumberFormat('en-US').format(count);
}
