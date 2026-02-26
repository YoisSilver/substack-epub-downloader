export type PublicationRequest = {
  url: string;
};

export type PublicationInfo = {
  url: string;
  title: string;
  author?: string;
  authorCoverUrl?: string;
};

export type PostSummary = {
  id: string;
  title: string;
  publishedAt: string;
  url: string;
  author?: string;
  coverImageUrl?: string;
  tags?: string[];
  subtitle?: string;
  summary?: string;
};

export type PublicationResponse = {
  publication: PublicationInfo;
  posts: PostSummary[];
};

export type ExportMode = "entire_profile" | "specific_posts";
export type OrderMode = "date" | "manual";
export type SortDirection = "desc" | "asc";
export type Format = "epub" | "txt";
export type Granularity = "per_post" | "combined";
export type CoverMode = "substack_author" | "custom";

export type MetadataField =
  | "title"
  | "author"
  | "publishedAt"
  | "url"
  | "tags"
  | "subtitle"
  | "readingTime"
  | "summary";

export type ExportJobRequest = {
  publicationUrl: string;
  publicationTitle: string;
  publicationAuthor?: string;
  authorCoverUrl?: string;
  mode: ExportMode;
  selectedPostIds: string[];
  orderMode: OrderMode;
  manualOrder: string[];
  sortDirection: SortDirection;
  formats: Format[];
  granularity: Granularity;
  coverMode: CoverMode;
  customCoverDataUrl?: string;
  metadataFields: MetadataField[];
  outputDir: string;
  posts: PostSummary[];
};

export type ExportJobResult = {
  succeeded: string[];
  failed: { postId: string; reason: string }[];
  outputFiles: string[];
  warnings: string[];
};

export type UserDefaults = {
  formats: Format[];
  granularity: Granularity;
  coverMode: CoverMode;
  metadataFields: MetadataField[];
};
