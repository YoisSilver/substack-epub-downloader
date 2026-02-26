import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { ChangeEvent, useEffect, useMemo, useState } from "react";
import type {
  CoverMode,
  Format,
  ExportJobRequest,
  ExportJobResult,
  ExportMode,
  Granularity,
  MetadataField,
  PostSummary,
  PublicationInfo,
  PublicationResponse,
  SortDirection,
  UserDefaults,
} from "./types";

type UiStep = "configure" | "reorder";

const STORAGE_KEY = "substack-downloader-defaults";

const METADATA_OPTIONS: Array<{ key: MetadataField; label: string }> = [
  { key: "title", label: "Title" },
  { key: "author", label: "Author" },
  { key: "publishedAt", label: "Publication date" },
  { key: "url", label: "Canonical URL" },
  { key: "tags", label: "Tags" },
  { key: "subtitle", label: "Subtitle" },
  { key: "readingTime", label: "Reading time" },
  { key: "summary", label: "Summary" },
];

const DEFAULT_SETTINGS: UserDefaults = {
  formats: ["epub", "txt"],
  granularity: "per_post",
  coverMode: "substack_author",
  metadataFields: ["title", "author", "publishedAt", "url", "tags", "subtitle", "summary"],
};

function loadDefaults(): UserDefaults {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) {
      return DEFAULT_SETTINGS;
    }
    const parsed = JSON.parse(raw) as Partial<UserDefaults>;
    return {
      formats: parsed.formats?.length ? parsed.formats : DEFAULT_SETTINGS.formats,
      granularity: parsed.granularity ?? DEFAULT_SETTINGS.granularity,
      coverMode: parsed.coverMode ?? DEFAULT_SETTINGS.coverMode,
      metadataFields: parsed.metadataFields?.length
        ? parsed.metadataFields
        : DEFAULT_SETTINGS.metadataFields,
    };
  } catch {
    return DEFAULT_SETTINGS;
  }
}

function parseDateForSort(value: string): number {
  const parsed = Date.parse(value);
  return Number.isNaN(parsed) ? 0 : parsed;
}

function sortPostsByDate(posts: PostSummary[], direction: SortDirection): PostSummary[] {
  return [...posts].sort((a, b) => {
    const aTs = parseDateForSort(a.publishedAt);
    const bTs = parseDateForSort(b.publishedAt);
    if (aTs !== bTs) {
      return direction === "desc" ? bTs - aTs : aTs - bTs;
    }
    return a.title.localeCompare(b.title);
  });
}

function mergeByExistingOrder(existingOrder: string[], nextIds: string[]): string[] {
  const nextSet = new Set(nextIds);
  const retained = existingOrder.filter((id) => nextSet.has(id));
  const newItems = nextIds.filter((id) => !retained.includes(id));
  return [...retained, ...newItems];
}

function formatDate(value: string): string {
  const parsed = new Date(value);
  return Number.isNaN(parsed.getTime()) ? value : parsed.toLocaleString();
}

function isDesktopRuntime(): boolean {
  return typeof window !== "undefined" && Boolean(window.__TAURI_INTERNALS__);
}

export default function App() {
  const desktop = isDesktopRuntime();
  const defaultSettings = useMemo(() => loadDefaults(), []);

  const [step, setStep] = useState<UiStep>("configure");
  const [publicationUrl, setPublicationUrl] = useState("");
  const [publication, setPublication] = useState<PublicationInfo | null>(null);
  const [posts, setPosts] = useState<PostSummary[]>([]);
  const [loadingPosts, setLoadingPosts] = useState(false);
  const [errorText, setErrorText] = useState<string | null>(null);

  const [mode, setMode] = useState<ExportMode>("entire_profile");
  const [sortDirection, setSortDirection] = useState<SortDirection>("desc");
  const [selectedPostIds, setSelectedPostIds] = useState<string[]>([]);
  const [finalOrder, setFinalOrder] = useState<string[]>([]);

  const [formats, setFormats] = useState<Format[]>(defaultSettings.formats);
  const [granularity, setGranularity] = useState<Granularity>(defaultSettings.granularity);
  const [coverMode, setCoverMode] = useState<CoverMode>(defaultSettings.coverMode);
  const [metadataFields, setMetadataFields] = useState<MetadataField[]>(defaultSettings.metadataFields);

  const [customCoverDataUrl, setCustomCoverDataUrl] = useState<string | undefined>(undefined);
  const [customCoverName, setCustomCoverName] = useState("");

  const [outputDir, setOutputDir] = useState("");
  const [exporting, setExporting] = useState(false);
  const [result, setResult] = useState<ExportJobResult | null>(null);

  const sortedPosts = useMemo(() => sortPostsByDate(posts, sortDirection), [posts, sortDirection]);
  const selectedPosts = useMemo(() => {
    const selectedSet = new Set(selectedPostIds);
    return sortedPosts.filter((post) => selectedSet.has(post.id));
  }, [sortedPosts, selectedPostIds]);
  const selectedOrderedPosts = useMemo(() => {
    const byId = new Map(posts.map((post) => [post.id, post]));
    return finalOrder.map((id) => byId.get(id)).filter((post): post is PostSummary => Boolean(post));
  }, [finalOrder, posts]);

  useEffect(() => {
    if (mode !== "specific_posts") {
      setStep("configure");
      setFinalOrder([]);
      return;
    }
    const nextIds = selectedPosts.map((post) => post.id);
    setFinalOrder((current) => mergeByExistingOrder(current, nextIds));
  }, [mode, selectedPosts]);

  function toggleFormat(format: Format) {
    setFormats((current) =>
      current.includes(format) ? current.filter((item) => item !== format) : [...current, format]
    );
  }

  function toggleMetadataField(field: MetadataField) {
    setMetadataFields((current) =>
      current.includes(field) ? current.filter((item) => item !== field) : [...current, field]
    );
  }

  function toggleSelectedPost(postId: string) {
    setSelectedPostIds((current) =>
      current.includes(postId) ? current.filter((item) => item !== postId) : [...current, postId]
    );
  }

  function movePost(postId: string, delta: number) {
    setFinalOrder((current) => {
      const index = current.indexOf(postId);
      if (index < 0) {
        return current;
      }
      const next = index + delta;
      if (next < 0 || next >= current.length) {
        return current;
      }
      const copy = [...current];
      [copy[index], copy[next]] = [copy[next], copy[index]];
      return copy;
    });
  }

  function setPostRank(postId: string, value: string) {
    const total = finalOrder.length;
    if (total === 0) {
      return;
    }
    const parsed = Number(value);
    if (!Number.isFinite(parsed)) {
      return;
    }
    const boundedRank = Math.max(1, Math.min(total, Math.trunc(parsed)));
    const targetIndex = total - boundedRank;
    setFinalOrder((current) => {
      const index = current.indexOf(postId);
      if (index < 0 || index === targetIndex) {
        return current;
      }
      const copy = [...current];
      copy.splice(index, 1);
      copy.splice(targetIndex, 0, postId);
      return copy;
    });
  }

  function advanceToReorder() {
    setErrorText(null);
    if (mode === "specific_posts") {
      if (selectedPostIds.length === 0) {
        setErrorText("Select at least one post.");
        return;
      }
      setFinalOrder(selectedPosts.map((post) => post.id));
      setStep("reorder");
    }
  }

  async function loadPublication() {
    setErrorText(null);
    setResult(null);
    setLoadingPosts(true);
    try {
      if (!desktop) {
        throw new Error("Use the desktop .exe. Browser mode is intentionally unsupported.");
      }
      const response = await invoke<PublicationResponse>("load_publication_posts", {
        request: { url: publicationUrl.trim() },
      });
      setPublication(response.publication);
      setPosts(response.posts);
      setSelectedPostIds([]);
      setFinalOrder([]);
      setStep("configure");
    } catch (error) {
      setErrorText(String(error));
      setPublication(null);
      setPosts([]);
      setSelectedPostIds([]);
      setFinalOrder([]);
      setStep("configure");
    } finally {
      setLoadingPosts(false);
    }
  }

  function saveCurrentDefaults() {
    const payload: UserDefaults = {
      formats,
      granularity,
      coverMode,
      metadataFields,
    };
    localStorage.setItem(STORAGE_KEY, JSON.stringify(payload));
  }

  async function pickOutputDirectory() {
    if (!desktop) {
      setErrorText("Folder picker works only in desktop .exe mode.");
      return;
    }
    const path = await open({ directory: true, multiple: false });
    if (typeof path === "string") {
      setOutputDir(path);
    }
  }

  async function handleCustomCoverChange(event: ChangeEvent<HTMLInputElement>) {
    const file = event.target.files?.[0];
    if (!file) {
      return;
    }
    const reader = new FileReader();
    reader.onload = () => {
      if (typeof reader.result === "string") {
        setCustomCoverDataUrl(reader.result);
        setCustomCoverName(file.name);
      }
    };
    reader.readAsDataURL(file);
  }

  async function runExport() {
    setErrorText(null);
    setResult(null);
    if (!desktop) {
      setErrorText("Export works only in desktop .exe mode. Web mode supports loading and arranging posts.");
      return;
    }
    if (!publication) {
      setErrorText("Load a publication before exporting.");
      return;
    }
    if (formats.length === 0) {
      setErrorText("Select at least one output format.");
      return;
    }
    if (!outputDir.trim()) {
      setErrorText("Select an output directory.");
      return;
    }
    if (mode === "specific_posts" && selectedPostIds.length === 0) {
      setErrorText("Select at least one post.");
      return;
    }
    if (mode === "specific_posts" && step !== "reorder") {
      setErrorText("Open final reorder step before exporting.");
      return;
    }
    if (coverMode === "custom" && !customCoverDataUrl && formats.includes("epub")) {
      setErrorText("Upload a custom cover image for EPUB export.");
      return;
    }

    const request: ExportJobRequest = {
      publicationUrl: publication.url,
      publicationTitle: publication.title,
      publicationAuthor: publication.author,
      authorCoverUrl: publication.authorCoverUrl,
      mode,
      selectedPostIds,
      orderMode: mode === "specific_posts" ? "manual" : "date",
      manualOrder: mode === "specific_posts" ? finalOrder : [],
      sortDirection,
      formats,
      granularity,
      coverMode,
      customCoverDataUrl,
      metadataFields,
      outputDir: outputDir.trim(),
      posts,
    };

    setExporting(true);
    try {
      const exportResult = await invoke<ExportJobResult>("run_export_job", { request });
      setResult(exportResult);
    } catch (error) {
      setErrorText(String(error));
    } finally {
      setExporting(false);
    }
  }

  return (
    <main className="app-shell">
      <header className="header">
        <h1>Substack Downloader</h1>
        <p>Desktop EPUB/TXT exporter for public Substack posts.</p>
      </header>

      <section className="panel">
        <div className="row">
          <label htmlFor="publication-url">Publication URL</label>
          <input
            id="publication-url"
            type="text"
            placeholder="https://example.substack.com"
            value={publicationUrl}
            onChange={(event) => setPublicationUrl(event.target.value)}
          />
          <button disabled={loadingPosts || !publicationUrl.trim()} onClick={loadPublication}>
            {loadingPosts ? "Loading..." : "Load Posts"}
          </button>
        </div>
        {publication && (
          <div className="publication-card">
            <h2>{publication.title}</h2>
            <p>
              {publication.author ? `Author: ${publication.author}` : "Author not found"} | Posts found:{" "}
              {posts.length}
            </p>
          </div>
        )}
      </section>

      <section className="layout">
        <article className="panel">
          <div className="panel-header">
            <h3>{step === "reorder" ? "Final Reorder" : "Post Selection"}</h3>
            <div className="inline-controls">
              <label>
                Mode
                <select
                  value={mode}
                  onChange={(event) => {
                    const next = event.target.value as ExportMode;
                    setMode(next);
                    if (next !== "specific_posts") {
                      setStep("configure");
                    }
                  }}
                >
                  <option value="entire_profile">Entire profile</option>
                  <option value="specific_posts">Specific posts</option>
                </select>
              </label>
              <label>
                Date sort
                <select value={sortDirection} onChange={(event) => setSortDirection(event.target.value as SortDirection)}>
                  <option value="desc">Newest first</option>
                  <option value="asc">Oldest first</option>
                </select>
              </label>
            </div>
          </div>

          {mode === "specific_posts" && step === "configure" && (
            <div className="row compact">
              <button onClick={() => setSelectedPostIds(sortedPosts.map((post) => post.id))}>Select all</button>
              <button onClick={() => setSelectedPostIds([])}>Clear selection</button>
              <span>{selectedPostIds.length} selected</span>
            </div>
          )}

          {step === "configure" && (
            <div className="post-list">
              {sortedPosts.map((post) => (
                <div key={post.id} className={`post-row ${selectedPostIds.includes(post.id) ? "selected" : ""}`}>
                  {mode === "specific_posts" ? (
                    <input
                      type="checkbox"
                      checked={selectedPostIds.includes(post.id)}
                      onChange={() => toggleSelectedPost(post.id)}
                    />
                  ) : (
                    <span className="index-pill">All</span>
                  )}
                  <div className="post-meta">
                    <strong>{post.title}</strong>
                    <span>{formatDate(post.publishedAt)}</span>
                  </div>
                </div>
              ))}
              {sortedPosts.length === 0 && <p>No posts loaded.</p>}
            </div>
          )}

          {step === "reorder" && mode === "specific_posts" && (
            <div className="post-list">
              <p className="help-text">Final compile order is from higher number to lower number.</p>
              {selectedOrderedPosts.map((post, index) => {
                const rank = selectedOrderedPosts.length - index;
                return (
                  <div key={post.id} className="post-row selected reorder-row">
                    <label className="rank-input">
                      <span>#</span>
                      <input
                        type="number"
                        min={1}
                        max={selectedOrderedPosts.length}
                        value={rank}
                        onChange={(event) => setPostRank(post.id, event.target.value)}
                      />
                    </label>
                    <div className="post-meta">
                      <strong>{post.title}</strong>
                      <span>{formatDate(post.publishedAt)}</span>
                    </div>
                    <div className="row compact">
                      <button type="button" onClick={() => movePost(post.id, -1)}>
                        Up
                      </button>
                      <button type="button" onClick={() => movePost(post.id, 1)}>
                        Down
                      </button>
                    </div>
                  </div>
                );
              })}
              {selectedOrderedPosts.length === 0 && <p>No selected posts for reorder.</p>}
            </div>
          )}
        </article>

        <article className="panel">
          <div className="panel-header">
            <h3>Export Settings</h3>
            <button onClick={saveCurrentDefaults}>Save Current as Defaults</button>
          </div>

          <div className="setting-grid">
            <div>
              <h4>Formats</h4>
              <label>
                <input type="checkbox" checked={formats.includes("epub")} onChange={() => toggleFormat("epub")} />
                EPUB
              </label>
              <label>
                <input type="checkbox" checked={formats.includes("txt")} onChange={() => toggleFormat("txt")} />
                TXT
              </label>
            </div>

            <div>
              <h4>Granularity</h4>
              <label>
                <input
                  type="radio"
                  checked={granularity === "per_post"}
                  onChange={() => setGranularity("per_post")}
                />
                One file per post
              </label>
              <label>
                <input
                  type="radio"
                  checked={granularity === "combined"}
                  onChange={() => setGranularity("combined")}
                />
                Single combined file
              </label>
            </div>

            <div>
              <h4>EPUB Cover</h4>
              <label>
                <input
                  type="radio"
                  checked={coverMode === "substack_author"}
                  onChange={() => setCoverMode("substack_author")}
                />
                Use Substack author cover + title page text
              </label>
              <label>
                <input type="radio" checked={coverMode === "custom"} onChange={() => setCoverMode("custom")} />
                Upload custom cover
              </label>
              {coverMode === "custom" && (
                <div className="row compact">
                  <input type="file" accept="image/*" onChange={handleCustomCoverChange} />
                  <span>{customCoverName || "No file selected"}</span>
                </div>
              )}
            </div>
          </div>

          <div>
            <h4>Metadata fields</h4>
            <div className="metadata-grid">
              {METADATA_OPTIONS.map((option) => (
                <label key={option.key}>
                  <input
                    type="checkbox"
                    checked={metadataFields.includes(option.key)}
                    onChange={() => toggleMetadataField(option.key)}
                  />
                  {option.label}
                </label>
              ))}
            </div>
          </div>

          <div className="row">
            <label htmlFor="output-dir">Output folder</label>
            <input
              id="output-dir"
              type="text"
              placeholder={"C:\\exports\\substack"}
              value={outputDir}
              onChange={(event) => setOutputDir(event.target.value)}
            />
            <button onClick={pickOutputDirectory}>Browse</button>
          </div>

          {mode === "specific_posts" && step === "configure" ? (
            <button className="primary" disabled={selectedPostIds.length === 0} onClick={advanceToReorder}>
              Next: Final Reorder
            </button>
          ) : (
            <>
              {mode === "specific_posts" && (
                <button type="button" onClick={() => setStep("configure")}>
                  Back to Selection
                </button>
              )}
              <button className="primary" disabled={exporting || posts.length === 0} onClick={runExport}>
                {exporting ? "Exporting..." : "Run Export"}
              </button>
            </>
          )}
        </article>
      </section>

      {errorText && (
        <section className="panel error">
          <strong>Error</strong>
          <p>{errorText}</p>
        </section>
      )}

      {result && (
        <section className="panel result">
          <h3>Export Result</h3>
          <p>Successful posts: {result.succeeded.length}</p>
          <p>Failed posts: {result.failed.length}</p>
          <p>Output files: {result.outputFiles.length}</p>
          {result.outputFiles.length > 0 && (
            <ul>
              {result.outputFiles.map((file) => (
                <li key={file}>{file}</li>
              ))}
            </ul>
          )}
          {result.failed.length > 0 && (
            <>
              <h4>Failures</h4>
              <ul>
                {result.failed.map((failure) => (
                  <li key={failure.postId}>
                    {failure.postId}: {failure.reason}
                  </li>
                ))}
              </ul>
            </>
          )}
          {result.warnings.length > 0 && (
            <>
              <h4>Warnings</h4>
              <ul>
                {result.warnings.map((warning) => (
                  <li key={warning}>{warning}</li>
                ))}
              </ul>
            </>
          )}
        </section>
      )}
    </main>
  );
}
