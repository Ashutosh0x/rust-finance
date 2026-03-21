// crates/polymarket/src/gamma.rs
// Gamma API client — events, markets, search, tags, series, comments, profiles
// Base URL: https://gamma-api.polymarket.com
// All endpoints are public (no authentication required)
// Rate Limits: 4,000 req/10s general, 500 req/10s /events, 300 req/10s /markets
// Docs: https://docs.polymarket.com/api-reference/introduction
// OpenAPI: https://docs.polymarket.com/api-spec/gamma-openapi.yaml

use reqwest::Client;
use serde::{Deserialize, Serialize};

// ─── Response Types (complete Polymarket Gamma OpenAPI spec) ─────

/// Optimized image metadata
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ImageOptimization {
    pub id: Option<String>,
    pub image_url_source: Option<String>,
    pub image_url_optimized: Option<String>,
    pub image_size_kb_source: Option<f64>,
    pub image_size_kb_optimized: Option<f64>,
    pub image_optimized_complete: Option<bool>,
    pub image_optimized_last_updated: Option<String>,
    #[serde(rename = "relID")]
    pub rel_id: Option<i64>,
    pub field: Option<String>,
    pub relname: Option<String>,
}

/// Tag
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GammaTag {
    pub id: Option<String>,
    pub label: Option<String>,
    pub slug: Option<String>,
    pub force_show: Option<bool>,
    pub force_hide: Option<bool>,
    pub is_carousel: Option<bool>,
    pub published_at: Option<String>,
    pub created_by: Option<serde_json::Value>,
    pub updated_by: Option<serde_json::Value>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

/// Category
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GammaCategory {
    pub id: Option<String>,
    pub label: Option<String>,
    pub slug: Option<String>,
    pub parent_category: Option<String>,
    pub published_at: Option<String>,
    pub created_by: Option<String>,
    pub updated_by: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

/// Series
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GammaSeries {
    pub id: Option<String>,
    pub ticker: Option<String>,
    pub slug: Option<String>,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub series_type: Option<String>,
    pub recurrence: Option<String>,
    pub description: Option<String>,
    pub image: Option<String>,
    pub icon: Option<String>,
    pub layout: Option<String>,
    pub active: Option<bool>,
    pub closed: Option<bool>,
    pub archived: Option<bool>,
    pub new: Option<bool>,
    pub featured: Option<bool>,
    pub restricted: Option<bool>,
    pub is_template: Option<bool>,
    pub template_variables: Option<serde_json::Value>,
    pub published_at: Option<String>,
    pub created_by: Option<String>,
    pub updated_by: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub comments_enabled: Option<bool>,
    pub competitive: Option<serde_json::Value>,
    #[serde(rename = "volume24hr")]
    pub volume_24hr: Option<f64>,
    pub volume: Option<f64>,
    pub liquidity: Option<f64>,
    pub start_date: Option<String>,
    #[serde(rename = "pythTokenID")]
    pub pyth_token_id: Option<String>,
    pub cg_asset_name: Option<String>,
    pub score: Option<i64>,
    pub comment_count: Option<i64>,
}

/// Collection
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GammaCollection {
    pub id: Option<String>,
    pub ticker: Option<String>,
    pub slug: Option<String>,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub collection_type: Option<String>,
    pub description: Option<String>,
    pub tags: Option<serde_json::Value>,
    pub image: Option<String>,
    pub icon: Option<String>,
    pub header_image: Option<String>,
    pub layout: Option<String>,
    pub active: Option<bool>,
    pub closed: Option<bool>,
    pub archived: Option<bool>,
    pub new: Option<bool>,
    pub featured: Option<bool>,
    pub restricted: Option<bool>,
    pub is_template: Option<bool>,
    pub template_variables: Option<String>,
    pub published_at: Option<String>,
    pub created_by: Option<String>,
    pub updated_by: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub comments_enabled: Option<bool>,
    pub image_optimized: Option<ImageOptimization>,
    pub icon_optimized: Option<ImageOptimization>,
    pub header_image_optimized: Option<ImageOptimization>,
}

/// Event creator
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventCreator {
    pub id: Option<String>,
    pub creator_name: Option<String>,
    pub creator_handle: Option<String>,
    pub creator_url: Option<String>,
    pub creator_image: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

/// Chat / live stream
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GammaChat {
    pub id: Option<String>,
    pub channel_id: Option<String>,
    pub channel_name: Option<String>,
    pub channel_image: Option<String>,
    pub live: Option<bool>,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
}

/// Template
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GammaTemplate {
    pub id: Option<String>,
    pub event_title: Option<String>,
    pub event_slug: Option<String>,
    pub event_image: Option<String>,
    pub market_title: Option<String>,
    pub description: Option<String>,
    pub resolution_source: Option<String>,
    pub neg_risk: Option<bool>,
    pub sort_by: Option<String>,
    pub show_market_images: Option<bool>,
    pub series_slug: Option<String>,
    pub outcomes: Option<String>,
}

/// Full Market — all fields from /api-spec/gamma-openapi.yaml Market schema
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GammaMarket {
    pub id: Option<String>,
    pub question: Option<String>,
    pub condition_id: Option<String>,
    pub slug: Option<String>,
    pub twitter_card_image: Option<String>,
    pub resolution_source: Option<String>,
    pub end_date: Option<String>,
    pub end_date_iso: Option<String>,
    pub start_date: Option<String>,
    pub start_date_iso: Option<String>,
    pub category: Option<String>,
    pub amm_type: Option<String>,
    pub liquidity: Option<String>,
    pub liquidity_num: Option<f64>,
    pub liquidity_amm: Option<f64>,
    pub liquidity_clob: Option<f64>,
    pub sponsor_name: Option<String>,
    pub sponsor_image: Option<String>,
    pub x_axis_value: Option<String>,
    pub y_axis_value: Option<String>,
    pub denomination_token: Option<String>,
    pub fee: Option<String>,
    pub image: Option<String>,
    pub icon: Option<String>,
    pub lower_bound: Option<String>,
    pub upper_bound: Option<String>,
    pub lower_bound_date: Option<String>,
    pub upper_bound_date: Option<String>,
    pub description: Option<String>,
    pub outcomes: Option<String>,
    pub outcome_prices: Option<String>,
    pub short_outcomes: Option<String>,
    pub volume: Option<String>,
    pub volume_num: Option<f64>,
    pub volume_amm: Option<f64>,
    pub volume_clob: Option<f64>,
    pub active: Option<bool>,
    pub closed: Option<bool>,
    pub market_type: Option<String>,
    pub format_type: Option<String>,
    pub market_maker_address: Option<String>,
    pub created_by: Option<serde_json::Value>,
    pub updated_by: Option<serde_json::Value>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub closed_time: Option<String>,
    pub wide_format: Option<bool>,
    pub new: Option<bool>,
    pub mailchimp_tag: Option<String>,
    pub featured: Option<bool>,
    pub archived: Option<bool>,
    pub resolved_by: Option<String>,
    pub restricted: Option<bool>,
    pub market_group: Option<i64>,
    pub group_item_title: Option<String>,
    pub group_item_threshold: Option<String>,
    pub group_item_range: Option<String>,
    #[serde(rename = "questionID")]
    pub question_id: Option<String>,
    pub uma_end_date: Option<String>,
    pub uma_end_date_iso: Option<String>,
    pub uma_resolution_status: Option<String>,
    pub uma_resolution_statuses: Option<String>,
    pub uma_bond: Option<String>,
    pub uma_reward: Option<String>,
    pub enable_order_book: Option<bool>,
    pub order_price_min_tick_size: Option<f64>,
    pub order_min_size: Option<f64>,
    pub curation_order: Option<i64>,
    pub has_reviewed_dates: Option<bool>,
    pub ready_for_cron: Option<bool>,
    pub comments_enabled: Option<bool>,
    #[serde(rename = "volume24hr")]
    pub volume_24hr: Option<f64>,
    #[serde(rename = "volume1wk")]
    pub volume_1wk: Option<f64>,
    #[serde(rename = "volume1mo")]
    pub volume_1mo: Option<f64>,
    #[serde(rename = "volume1yr")]
    pub volume_1yr: Option<f64>,
    #[serde(rename = "volume24hrAmm")]
    pub volume_24hr_amm: Option<f64>,
    #[serde(rename = "volume1wkAmm")]
    pub volume_1wk_amm: Option<f64>,
    #[serde(rename = "volume1moAmm")]
    pub volume_1mo_amm: Option<f64>,
    #[serde(rename = "volume1yrAmm")]
    pub volume_1yr_amm: Option<f64>,
    #[serde(rename = "volume24hrClob")]
    pub volume_24hr_clob: Option<f64>,
    #[serde(rename = "volume1wkClob")]
    pub volume_1wk_clob: Option<f64>,
    #[serde(rename = "volume1moClob")]
    pub volume_1mo_clob: Option<f64>,
    #[serde(rename = "volume1yrClob")]
    pub volume_1yr_clob: Option<f64>,
    pub game_start_time: Option<String>,
    pub seconds_delay: Option<i64>,
    pub clob_token_ids: Option<String>,
    pub disqus_thread: Option<String>,
    #[serde(rename = "teamAID")]
    pub team_a_id: Option<String>,
    #[serde(rename = "teamBID")]
    pub team_b_id: Option<String>,
    pub fpmm_live: Option<bool>,
    pub maker_base_fee: Option<i64>,
    pub taker_base_fee: Option<i64>,
    pub custom_liveness: Option<i64>,
    pub accepting_orders: Option<bool>,
    pub notifications_enabled: Option<bool>,
    pub score: Option<i64>,
    pub neg_risk: Option<bool>,
    pub neg_risk_other: Option<bool>,
    pub best_bid: Option<f64>,
    pub best_ask: Option<f64>,
    pub last_trade_price: Option<f64>,
    pub spread: Option<f64>,
    pub one_day_price_change: Option<f64>,
    pub one_hour_price_change: Option<f64>,
    pub one_week_price_change: Option<f64>,
    pub one_month_price_change: Option<f64>,
    pub one_year_price_change: Option<f64>,
    pub competitive: Option<f64>,
    pub rewards_min_size: Option<f64>,
    pub rewards_max_spread: Option<f64>,
    pub automatically_resolved: Option<bool>,
    pub automatically_active: Option<bool>,
    pub clear_book_on_start: Option<bool>,
    pub chart_color: Option<String>,
    pub series_color: Option<String>,
    pub show_gmp_series: Option<bool>,
    pub show_gmp_outcome: Option<bool>,
    pub manual_activation: Option<bool>,
    pub creator: Option<String>,
    pub ready: Option<bool>,
    pub funded: Option<bool>,
    pub past_slugs: Option<String>,
    pub ready_timestamp: Option<String>,
    pub funded_timestamp: Option<String>,
    pub accepting_orders_timestamp: Option<String>,
    pub rfq_enabled: Option<bool>,
    pub game_id: Option<String>,
    pub sports_market_type: Option<String>,
    pub line: Option<f64>,
    pub pending_deployment: Option<bool>,
    pub deploying: Option<bool>,
    pub deploying_timestamp: Option<String>,
    pub scheduled_deployment_timestamp: Option<String>,
    pub event_start_time: Option<String>,
    pub image_optimized: Option<ImageOptimization>,
    pub icon_optimized: Option<ImageOptimization>,
    pub events: Option<Vec<serde_json::Value>>,
    pub categories: Option<Vec<GammaCategory>>,
    pub tags: Option<Vec<GammaTag>>,
}

/// Full Event — all fields from /api-spec/gamma-openapi.yaml Event schema
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GammaEvent {
    pub id: Option<String>,
    pub ticker: Option<String>,
    pub slug: Option<String>,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub description: Option<String>,
    pub resolution_source: Option<String>,
    pub start_date: Option<String>,
    pub creation_date: Option<String>,
    pub end_date: Option<String>,
    pub image: Option<String>,
    pub icon: Option<String>,
    pub active: Option<bool>,
    pub closed: Option<bool>,
    pub archived: Option<bool>,
    pub new: Option<bool>,
    pub featured: Option<bool>,
    pub restricted: Option<bool>,
    pub liquidity: Option<f64>,
    pub liquidity_amm: Option<f64>,
    pub liquidity_clob: Option<f64>,
    pub volume: Option<f64>,
    pub open_interest: Option<f64>,
    pub sort_by: Option<String>,
    pub category: Option<String>,
    pub subcategory: Option<String>,
    pub is_template: Option<bool>,
    pub template_variables: Option<String>,
    pub published_at: Option<String>,
    pub created_by: Option<String>,
    pub updated_by: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub comments_enabled: Option<bool>,
    pub competitive: Option<f64>,
    #[serde(rename = "volume24hr")]
    pub volume_24hr: Option<f64>,
    #[serde(rename = "volume1wk")]
    pub volume_1wk: Option<f64>,
    #[serde(rename = "volume1mo")]
    pub volume_1mo: Option<f64>,
    #[serde(rename = "volume1yr")]
    pub volume_1yr: Option<f64>,
    pub featured_image: Option<String>,
    pub disqus_thread: Option<String>,
    pub parent_event: Option<String>,
    pub enable_order_book: Option<bool>,
    pub neg_risk: Option<bool>,
    #[serde(rename = "negRiskMarketID")]
    pub neg_risk_market_id: Option<String>,
    pub neg_risk_fee_bips: Option<i64>,
    pub enable_neg_risk: Option<bool>,
    pub comment_count: Option<i64>,
    pub sub_events: Option<Vec<String>>,
    pub cyom: Option<bool>,
    pub closed_time: Option<String>,
    pub show_all_outcomes: Option<bool>,
    pub show_market_images: Option<bool>,
    pub automatically_resolved: Option<bool>,
    pub automatically_active: Option<bool>,
    pub event_date: Option<String>,
    pub start_time: Option<String>,
    pub event_week: Option<i64>,
    pub series_slug: Option<String>,
    pub score: Option<String>,
    pub elapsed: Option<String>,
    pub period: Option<String>,
    pub live: Option<bool>,
    pub ended: Option<bool>,
    pub finished_timestamp: Option<String>,
    pub gmp_chart_mode: Option<String>,
    pub tweet_count: Option<i64>,
    pub featured_order: Option<i64>,
    pub estimate_value: Option<bool>,
    pub cant_estimate: Option<bool>,
    pub estimated_value: Option<String>,
    pub spreads_main_line: Option<f64>,
    pub totals_main_line: Option<f64>,
    pub carousel_map: Option<String>,
    pub pending_deployment: Option<bool>,
    pub deploying: Option<bool>,
    pub deploying_timestamp: Option<String>,
    pub scheduled_deployment_timestamp: Option<String>,
    pub game_status: Option<String>,
    pub markets: Option<Vec<GammaMarket>>,
    pub tags: Option<Vec<GammaTag>>,
    pub categories: Option<Vec<GammaCategory>>,
    pub series: Option<Vec<GammaSeries>>,
    pub collections: Option<Vec<GammaCollection>>,
    pub event_creators: Option<Vec<EventCreator>>,
    pub chats: Option<Vec<GammaChat>>,
    pub templates: Option<Vec<GammaTemplate>>,
    pub image_optimized: Option<ImageOptimization>,
    pub icon_optimized: Option<ImageOptimization>,
    pub featured_image_optimized: Option<ImageOptimization>,
}

/// Comment
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GammaComment {
    pub id: Option<String>,
    pub body: Option<String>,
    pub parent_entity_type: Option<String>,
    #[serde(rename = "parentEntityID")]
    pub parent_entity_id: Option<i64>,
    #[serde(rename = "parentCommentID")]
    pub parent_comment_id: Option<String>,
    pub user_address: Option<String>,
    pub reply_address: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub profile: Option<CommentProfile>,
    pub reactions: Option<Vec<Reaction>>,
    pub report_count: Option<i64>,
    pub reaction_count: Option<i64>,
}

/// Comment author profile
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommentProfile {
    pub name: Option<String>,
    pub pseudonym: Option<String>,
    pub display_username_public: Option<bool>,
    pub bio: Option<String>,
    pub is_mod: Option<bool>,
    pub is_creator: Option<bool>,
    pub proxy_wallet: Option<String>,
    pub base_address: Option<String>,
    pub profile_image: Option<String>,
    pub profile_image_optimized: Option<ImageOptimization>,
    pub positions: Option<Vec<CommentPosition>>,
}

/// Reaction on a comment
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reaction {
    pub id: Option<String>,
    #[serde(rename = "commentID")]
    pub comment_id: Option<i64>,
    pub reaction_type: Option<String>,
    pub icon: Option<String>,
    pub user_address: Option<String>,
    pub created_at: Option<String>,
    pub profile: Option<CommentProfile>,
}

/// Comment position
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommentPosition {
    pub token_id: Option<String>,
    pub position_size: Option<String>,
}

/// Public profile
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicProfile {
    pub created_at: Option<String>,
    pub proxy_wallet: Option<String>,
    pub profile_image: Option<String>,
    pub display_username_public: Option<bool>,
    pub bio: Option<String>,
    pub pseudonym: Option<String>,
    pub name: Option<String>,
    pub x_username: Option<String>,
    pub verified_badge: Option<bool>,
    pub users: Option<Vec<PublicProfileUser>>,
}

/// User associated with a public profile
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicProfileUser {
    pub id: Option<String>,
    pub creator: Option<bool>,
    #[serde(rename = "mod")]
    pub is_mod: Option<bool>,
}

/// Token helper
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Token {
    pub token_id: String,
    pub outcome: String,
    pub price: Option<f64>,
    pub winner: Option<bool>,
}

// ─── Query builders (matching full OpenAPI query params) ─────────

/// Builder for /events query parameters
#[derive(Debug, Default, Clone)]
pub struct EventQuery {
    pub active: Option<bool>,
    pub closed: Option<bool>,
    pub archived: Option<bool>,
    pub featured: Option<bool>,
    pub cyom: Option<bool>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    pub order: Option<String>,
    pub ascending: Option<bool>,
    pub tag_id: Option<i64>,
    pub exclude_tag_id: Option<Vec<i64>>,
    pub tag_slug: Option<String>,
    pub related_tags: Option<bool>,
    pub include_chat: Option<bool>,
    pub include_template: Option<bool>,
    pub recurrence: Option<String>,
    pub liquidity_min: Option<f64>,
    pub liquidity_max: Option<f64>,
    pub volume_min: Option<f64>,
    pub volume_max: Option<f64>,
    pub start_date_min: Option<String>,
    pub start_date_max: Option<String>,
    pub end_date_min: Option<String>,
    pub end_date_max: Option<String>,
    pub slug: Option<Vec<String>>,
    pub id: Option<Vec<i64>>,
}

impl EventQuery {
    pub fn new() -> Self { Self::default() }

    fn to_params(&self) -> Vec<(String, String)> {
        let mut p = Vec::new();
        macro_rules! push_opt {
            ($field:ident) => {
                if let Some(ref v) = self.$field { p.push((stringify!($field).to_string(), v.to_string())); }
            };
        }
        push_opt!(active); push_opt!(closed); push_opt!(archived);
        push_opt!(featured); push_opt!(cyom); push_opt!(limit);
        push_opt!(offset); push_opt!(order); push_opt!(ascending);
        push_opt!(tag_id); push_opt!(tag_slug); push_opt!(related_tags);
        push_opt!(include_chat); push_opt!(include_template);
        push_opt!(recurrence);
        push_opt!(liquidity_min); push_opt!(liquidity_max);
        push_opt!(volume_min); push_opt!(volume_max);
        push_opt!(start_date_min); push_opt!(start_date_max);
        push_opt!(end_date_min); push_opt!(end_date_max);
        if let Some(ref ids) = self.exclude_tag_id {
            for id in ids { p.push(("exclude_tag_id".to_string(), id.to_string())); }
        }
        if let Some(ref slugs) = self.slug {
            for s in slugs { p.push(("slug".to_string(), s.clone())); }
        }
        if let Some(ref ids) = self.id {
            for id in ids { p.push(("id".to_string(), id.to_string())); }
        }
        p
    }
}

/// Builder for /markets query parameters
#[derive(Debug, Default, Clone)]
pub struct MarketQuery {
    pub active: Option<bool>,
    pub closed: Option<bool>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    pub order: Option<String>,
    pub ascending: Option<bool>,
    pub tag_id: Option<i64>,
    pub related_tags: Option<bool>,
    pub cyom: Option<bool>,
    pub include_tag: Option<bool>,
    pub clob_token_ids: Option<Vec<String>>,
    pub condition_ids: Option<Vec<String>>,
    pub slug: Option<Vec<String>>,
    pub id: Option<Vec<i64>>,
    pub market_maker_address: Option<Vec<String>>,
    pub question_ids: Option<Vec<String>>,
    pub liquidity_num_min: Option<f64>,
    pub liquidity_num_max: Option<f64>,
    pub volume_num_min: Option<f64>,
    pub volume_num_max: Option<f64>,
    pub start_date_min: Option<String>,
    pub start_date_max: Option<String>,
    pub end_date_min: Option<String>,
    pub end_date_max: Option<String>,
    pub uma_resolution_status: Option<String>,
    pub rewards_min_size: Option<f64>,
    pub game_id: Option<String>,
    pub sports_market_types: Option<Vec<String>>,
}

impl MarketQuery {
    pub fn new() -> Self { Self::default() }

    fn to_params(&self) -> Vec<(String, String)> {
        let mut p = Vec::new();
        macro_rules! push_opt {
            ($field:ident) => {
                if let Some(ref v) = self.$field { p.push((stringify!($field).to_string(), v.to_string())); }
            };
        }
        push_opt!(active); push_opt!(closed); push_opt!(limit);
        push_opt!(offset); push_opt!(order); push_opt!(ascending);
        push_opt!(tag_id); push_opt!(related_tags); push_opt!(cyom);
        push_opt!(include_tag);
        push_opt!(liquidity_num_min); push_opt!(liquidity_num_max);
        push_opt!(volume_num_min); push_opt!(volume_num_max);
        push_opt!(start_date_min); push_opt!(start_date_max);
        push_opt!(end_date_min); push_opt!(end_date_max);
        push_opt!(uma_resolution_status); push_opt!(rewards_min_size);
        push_opt!(game_id);
        if let Some(ref ids) = self.clob_token_ids {
            for id in ids { p.push(("clob_token_ids".to_string(), id.clone())); }
        }
        if let Some(ref ids) = self.condition_ids {
            for id in ids { p.push(("condition_ids".to_string(), id.clone())); }
        }
        if let Some(ref slugs) = self.slug {
            for s in slugs { p.push(("slug".to_string(), s.clone())); }
        }
        if let Some(ref ids) = self.id {
            for id in ids { p.push(("id".to_string(), id.to_string())); }
        }
        if let Some(ref addrs) = self.market_maker_address {
            for a in addrs { p.push(("market_maker_address".to_string(), a.clone())); }
        }
        if let Some(ref ids) = self.question_ids {
            for id in ids { p.push(("question_ids".to_string(), id.clone())); }
        }
        if let Some(ref types) = self.sports_market_types {
            for t in types { p.push(("sports_market_types".to_string(), t.clone())); }
        }
        p
    }
}

// ─── Client ─────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct GammaClient {
    http: Client,
    base_url: String,
}

impl GammaClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            http: Client::new(),
            base_url: base_url.to_string(),
        }
    }

    // ═══════════════════════════════════════════════════════════════
    // ─── Events API ─────────────────────────────────────────────
    // ═══════════════════════════════════════════════════════════════

    /// GET /events — list events with full query builder
    pub async fn query_events(&self, query: &EventQuery) -> anyhow::Result<Vec<GammaEvent>> {
        let resp = self.http
            .get(format!("{}/events", self.base_url))
            .query(&query.to_params())
            .send()
            .await?
            .json::<Vec<GammaEvent>>()
            .await?;
        Ok(resp)
    }

    /// GET /events — simple list with basic filters
    pub async fn list_events(
        &self,
        active: Option<bool>,
        closed: Option<bool>,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> anyhow::Result<Vec<GammaEvent>> {
        let q = EventQuery {
            active, closed, limit, offset,
            ..Default::default()
        };
        self.query_events(&q).await
    }

    /// GET /events/{id} — get event by numeric ID
    pub async fn get_event(&self, event_id: &str) -> anyhow::Result<GammaEvent> {
        let resp = self.http
            .get(format!("{}/events/{}", self.base_url, event_id))
            .send()
            .await?
            .json::<GammaEvent>()
            .await?;
        Ok(resp)
    }

    /// GET /events/slug/{slug} — get event by slug
    pub async fn get_event_by_slug(&self, slug: &str) -> anyhow::Result<GammaEvent> {
        let resp = self.http
            .get(format!("{}/events/slug/{}", self.base_url, slug))
            .send()
            .await?
            .json::<GammaEvent>()
            .await?;
        Ok(resp)
    }

    /// GET /events/{id}/tags — get tags for an event
    pub async fn get_event_tags(&self, event_id: &str) -> anyhow::Result<Vec<GammaTag>> {
        let resp = self.http
            .get(format!("{}/events/{}/tags", self.base_url, event_id))
            .send()
            .await?
            .json::<Vec<GammaTag>>()
            .await?;
        Ok(resp)
    }

    // ═══════════════════════════════════════════════════════════════
    // ─── Markets API ────────────────────────────────────────────
    // ═══════════════════════════════════════════════════════════════

    /// GET /markets — list markets with full query builder
    pub async fn query_markets(&self, query: &MarketQuery) -> anyhow::Result<Vec<GammaMarket>> {
        let resp = self.http
            .get(format!("{}/markets", self.base_url))
            .query(&query.to_params())
            .send()
            .await?
            .json::<Vec<GammaMarket>>()
            .await?;
        Ok(resp)
    }

    /// GET /markets — fetch active, open markets
    pub async fn get_active_markets(
        &self,
        limit: u32,
        offset: u32,
    ) -> anyhow::Result<Vec<GammaMarket>> {
        let q = MarketQuery {
            active: Some(true),
            closed: Some(false),
            limit: Some(limit),
            offset: Some(offset),
            ..Default::default()
        };
        self.query_markets(&q).await
    }

    /// GET /markets/{id} — get market by numeric ID
    pub async fn get_market(&self, market_id: &str) -> anyhow::Result<GammaMarket> {
        let resp = self.http
            .get(format!("{}/markets/{}", self.base_url, market_id))
            .send()
            .await?
            .json::<GammaMarket>()
            .await?;
        Ok(resp)
    }

    /// GET /markets/slug/{slug} — get market by slug
    pub async fn get_market_by_slug(&self, slug: &str) -> anyhow::Result<GammaMarket> {
        let resp = self.http
            .get(format!("{}/markets/slug/{}", self.base_url, slug))
            .send()
            .await?
            .json::<GammaMarket>()
            .await?;
        Ok(resp)
    }

    /// GET /markets/{id}/tags — get tags for a market
    pub async fn get_market_tags(&self, market_id: &str) -> anyhow::Result<Vec<GammaTag>> {
        let resp = self.http
            .get(format!("{}/markets/{}/tags", self.base_url, market_id))
            .send()
            .await?
            .json::<Vec<GammaTag>>()
            .await?;
        Ok(resp)
    }

    // ═══════════════════════════════════════════════════════════════
    // ─── Comments API ───────────────────────────────────────────
    // ═══════════════════════════════════════════════════════════════

    /// GET /comments — list comments with filters
    pub async fn list_comments(
        &self,
        parent_entity_type: Option<&str>,
        parent_entity_id: Option<i64>,
        limit: Option<u32>,
        offset: Option<u32>,
        holders_only: Option<bool>,
    ) -> anyhow::Result<Vec<GammaComment>> {
        let mut params: Vec<(String, String)> = Vec::new();
        if let Some(t) = parent_entity_type { params.push(("parent_entity_type".to_string(), t.to_string())); }
        if let Some(id) = parent_entity_id { params.push(("parent_entity_id".to_string(), id.to_string())); }
        if let Some(l) = limit { params.push(("limit".to_string(), l.to_string())); }
        if let Some(o) = offset { params.push(("offset".to_string(), o.to_string())); }
        if let Some(h) = holders_only { params.push(("holders_only".to_string(), h.to_string())); }

        let resp = self.http
            .get(format!("{}/comments", self.base_url))
            .query(&params)
            .send()
            .await?
            .json::<Vec<GammaComment>>()
            .await?;
        Ok(resp)
    }

    /// GET /comments/{id} — get comment by ID
    pub async fn get_comment(&self, comment_id: &str) -> anyhow::Result<GammaComment> {
        let resp = self.http
            .get(format!("{}/comments/{}", self.base_url, comment_id))
            .send()
            .await?
            .json::<GammaComment>()
            .await?;
        Ok(resp)
    }

    /// GET /comments?user_address={addr} — get comments by user
    pub async fn get_comments_by_user(&self, address: &str) -> anyhow::Result<Vec<GammaComment>> {
        let resp = self.http
            .get(format!("{}/comments", self.base_url))
            .query(&[("user_address", address)])
            .send()
            .await?
            .json::<Vec<GammaComment>>()
            .await?;
        Ok(resp)
    }

    // ═══════════════════════════════════════════════════════════════
    // ─── Profiles API ───────────────────────────────────────────
    // ═══════════════════════════════════════════════════════════════

    /// GET /public-profile?address={addr} — get public profile by wallet address
    pub async fn get_public_profile(&self, address: &str) -> anyhow::Result<PublicProfile> {
        let resp = self.http
            .get(format!("{}/public-profile", self.base_url))
            .query(&[("address", address)])
            .send()
            .await?
            .json::<PublicProfile>()
            .await?;
        Ok(resp)
    }

    // ═══════════════════════════════════════════════════════════════
    // ─── Search API ─────────────────────────────────────────────
    // ═══════════════════════════════════════════════════════════════

    /// GET /search — search markets, events, profiles
    pub async fn search(&self, query: &str) -> anyhow::Result<serde_json::Value> {
        let resp = self.http
            .get(format!("{}/search", self.base_url))
            .query(&[("q", query)])
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;
        Ok(resp)
    }

    /// GET /public-search — search markets and events (public endpoint)
    pub async fn public_search(&self, query: &str) -> anyhow::Result<serde_json::Value> {
        let resp = self.http
            .get(format!("{}/public-search", self.base_url))
            .query(&[("q", query)])
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;
        Ok(resp)
    }

    // ═══════════════════════════════════════════════════════════════
    // ─── Tags API ───────────────────────────────────────────────
    // ═══════════════════════════════════════════════════════════════

    /// GET /tags — list all tags
    pub async fn list_tags(&self) -> anyhow::Result<Vec<GammaTag>> {
        let resp = self.http
            .get(format!("{}/tags", self.base_url))
            .send()
            .await?
            .json::<Vec<GammaTag>>()
            .await?;
        Ok(resp)
    }

    /// GET /tags/{id} — get tag by ID
    pub async fn get_tag(&self, tag_id: &str) -> anyhow::Result<GammaTag> {
        let resp = self.http
            .get(format!("{}/tags/{}", self.base_url, tag_id))
            .send()
            .await?
            .json::<GammaTag>()
            .await?;
        Ok(resp)
    }

    // ═══════════════════════════════════════════════════════════════
    // ─── Convenience ────────────────────────────────────────────
    // ═══════════════════════════════════════════════════════════════

    /// Get BTC 15-min up/down events
    pub async fn get_btc_15min_events(&self, limit: u32) -> anyhow::Result<Vec<GammaEvent>> {
        let q = EventQuery {
            active: Some(true),
            closed: Some(false),
            limit: Some(limit),
            order: Some("volume24hr".to_string()),
            ascending: Some(false),
            ..Default::default()
        };
        let all = self.query_events(&q).await?;
        Ok(all
            .into_iter()
            .filter(|e| {
                e.slug.as_deref()
                    .unwrap_or("")
                    .starts_with("btc-updown-15m")
            })
            .collect())
    }

    /// Search markets by query string (client-side filter)
    pub async fn search_markets(&self, query: &str) -> anyhow::Result<Vec<GammaMarket>> {
        let q = MarketQuery {
            closed: Some(false),
            limit: Some(50),
            ..Default::default()
        };
        let resp = self.query_markets(&q).await?;
        Ok(resp
            .into_iter()
            .filter(|m| {
                m.question.as_deref()
                    .unwrap_or("")
                    .to_lowercase()
                    .contains(&query.to_lowercase())
            })
            .collect())
    }

    /// Get trending events (sorted by 24h volume, active only)
    pub async fn get_trending_events(&self, limit: u32) -> anyhow::Result<Vec<GammaEvent>> {
        let q = EventQuery {
            active: Some(true),
            closed: Some(false),
            limit: Some(limit),
            order: Some("volume24hr".to_string()),
            ascending: Some(false),
            ..Default::default()
        };
        self.query_events(&q).await
    }

    /// Get crypto events (by tag slug)
    pub async fn get_crypto_events(&self, limit: u32) -> anyhow::Result<Vec<GammaEvent>> {
        let q = EventQuery {
            active: Some(true),
            closed: Some(false),
            limit: Some(limit),
            tag_slug: Some("crypto".to_string()),
            order: Some("volume24hr".to_string()),
            ascending: Some(false),
            ..Default::default()
        };
        self.query_events(&q).await
    }
}
