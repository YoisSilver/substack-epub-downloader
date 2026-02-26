use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicationRequest {
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicationInfo {
    pub url: String,
    pub title: String,
    pub author: Option<String>,
    pub author_cover_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PostSummary {
    pub id: String,
    pub title: String,
    pub published_at: String,
    pub url: String,
    pub author: Option<String>,
    pub cover_image_url: Option<String>,
    pub tags: Option<Vec<String>>,
    pub subtitle: Option<String>,
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicationResponse {
    pub publication: PublicationInfo,
    pub posts: Vec<PostSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportMode {
    EntireProfile,
    SpecificPosts,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderMode {
    Date,
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortDirection {
    Desc,
    Asc,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Epub,
    Txt,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Granularity {
    PerPost,
    Combined,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverMode {
    SubstackAuthor,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum MetadataField {
    Title,
    Author,
    PublishedAt,
    Url,
    Tags,
    Subtitle,
    ReadingTime,
    Summary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportJobRequest {
    pub publication_url: String,
    pub publication_title: String,
    pub publication_author: Option<String>,
    pub author_cover_url: Option<String>,
    pub mode: ExportMode,
    pub selected_post_ids: Vec<String>,
    pub order_mode: OrderMode,
    pub manual_order: Vec<String>,
    pub sort_direction: SortDirection,
    pub formats: Vec<ExportFormat>,
    pub granularity: Granularity,
    pub cover_mode: CoverMode,
    pub custom_cover_data_url: Option<String>,
    pub metadata_fields: Vec<MetadataField>,
    pub output_dir: String,
    pub posts: Vec<PostSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportFailure {
    pub post_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportJobResult {
    pub succeeded: Vec<String>,
    pub failed: Vec<ExportFailure>,
    pub output_files: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PostContent {
    pub summary: PostSummary,
    pub plain_text: String,
    pub epub_body: String,
    pub reading_time_minutes: Option<u32>,
    pub summary_text: Option<String>,
}
