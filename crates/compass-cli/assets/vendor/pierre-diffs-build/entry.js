/*! @pierre/diffs v1.2.12 | Apache-2.0 | Modified by Compass: plain-text Shiki bundle and global browser facade */
import { FileDiff } from "./node_modules/@pierre/diffs/dist/components/FileDiff.js";
import { parsePatchFiles } from "./node_modules/@pierre/diffs/dist/utils/parsePatchFiles.js";

globalThis.CompassDiffs = { FileDiff, parsePatchFiles };
