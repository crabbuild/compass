"""Frozen common name-focus policy for paired public-member evaluation.

Both tools use these same functions. No source body, graph file, witness,
language-specific synonym, or tool-specific metadata is consulted to rank.
"""
from collections import defaultdict
import json
from pathlib import Path
import re
import unicodedata

from benchmarks.agent_query.community_tasks import read_bounded
from benchmarks.agent_query.source_windows import MAX_ROWS

_LEXICON = json.loads(read_bounded(Path(__file__).with_name('member_focus_lexicon.json'), 65536))
if _LEXICON['schema'] != 'compass.member-focus-lexicon/1':
    raise ValueError('unsupported member focus lexicon')
_STOP = frozenset(_LEXICON['stopwords'])
_CANONICAL = _LEXICON['canonicalTokens']


def identifier_tokens(text):
    text = ''.join(c for c in unicodedata.normalize('NFKD', text)
                   if unicodedata.category(c) not in {'Mn', 'Mc', 'Me'})
    out = []
    for i, character in enumerate(text):
        previous = text[i - 1] if i else ''
        following = text[i + 1] if i + 1 < len(text) else ''
        if character.isupper() and previous and (
                previous.islower() or previous.isnumeric()
                or (previous.isupper() and following.islower())):
            out.append(' ')
        out.append(character)
    return re.findall(r'\w+', ''.join(out).lower())


def _ending(stem):
    if len(stem) > 1 and stem[-1] == stem[-2]:
        stem = stem[:-1]
    return stem + 'e' if stem.endswith(('at', 'abl', 'il', 'v')) else stem


def canonical(token):
    if not token.isascii():
        return token
    if token in _CANONICAL:
        return _CANONICAL[token]
    if token.endswith('ies') and len(token[:-3]) >= 2:
        return token[:-3] + 'y'
    if token.endswith(('sses', 'xes', 'zes', 'ches', 'shes', 'uses')):
        return token[:-2]
    if (token.endswith('s') and len(token[:-1]) >= 3
            and not token[:-1].endswith(('s', 'u', 'i', 'a'))):
        return token[:-1]
    if token.endswith('ing') and len(token[:-3]) >= 3:
        return _ending(token[:-3])
    if token.endswith('ied') and len(token[:-3]) >= 2:
        return token[:-3] + 'y'
    if token.endswith('ed') and len(token[:-2]) >= 3:
        return _ending(token[:-2])
    return token


def _searchable(token):
    return not all('a' <= c <= 'z' for c in token) or len(token) > 2


def focus_terms(question):
    if not isinstance(question, str) or len(question.encode()) > 4096:
        raise ValueError('focus must be text within 4096 bytes')
    raw = []
    for word in question.split():
        if any('\u4e00' <= c <= '\u9fff' for c in word):
            lowered = word.lower()
            if len(lowered) < 2:
                if _searchable(lowered):
                    raw.append(lowered)
            else:
                raw.extend(lowered[i:i + 2] for i in range(len(lowered) - 1)
                           if _searchable(lowered[i:i + 2]))
                if _searchable(lowered) and lowered not in raw:
                    raw.append(lowered)
        else:
            raw.extend(t for t in identifier_tokens(word) if _searchable(t))
    content = [term for term in raw if term not in _STOP]
    result = sorted({term if term in _STOP else canonical(term) for term in content or raw})
    if not 1 <= len(result) <= 32:
        raise ValueError('focus requires 1 to 32 distinct searchable terms')
    return result


def name_terms(name):
    result = set()
    for token in identifier_tokens(name):
        result.add(canonical(token))
        if '_' in token:
            result.update(canonical(part) for part in token.split('_') if part)
    return result


def focus_groups(rows, question):
    terms = focus_terms(question)
    if not isinstance(rows, list) or len(rows) > MAX_ROWS:
        raise ValueError('membership row limit exceeded')
    groups = defaultdict(set)
    total = 0
    for row in rows:
        if not isinstance(row, dict):
            raise ValueError('invalid membership row')
        label, file, line = row.get('label'), row.get('file'), row.get('line')
        if (not isinstance(label, str) or not label or len(label.encode()) > 4096
                or not isinstance(file, str) or not file or type(line) is not int or line < 1):
            raise ValueError('invalid membership label or source anchor')
        total += len(label.encode()) + len(file.encode())
        if total > 1048576:
            raise ValueError('membership label metadata limit exceeded')
        groups[file, line].add(label)
    ranked = []
    for (file, line), labels in sorted(groups.items()):
        matches = [dict(label=label, matchedTerms=sorted(name_terms(label) & set(terms)))
                   for label in sorted(labels)]
        ranked.append(dict(file=file, line=line, score=max(len(m['matchedTerms']) for m in matches),
                           labels=matches))
    ranked.sort(key=lambda group: (-group['score'], group['file'], group['line']))
    return dict(focusTerms=terms, groups=ranked)
