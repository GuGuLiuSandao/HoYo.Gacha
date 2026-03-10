use hg_metadata::Entry;
use serde::Serialize;
use serde_json::Value as JsonValue;
use snafu::{OptionExt, Snafu};
use time::format_description::well_known::Rfc3339;
use time::serde::rfc3339;
use time::{Duration, OffsetDateTime};

use crate::bootstrap::{TauriDatabaseState, TauriMetadataState};
use crate::database::schemas::{
  AccountBusiness, GachaRecord, GachaRecordSaveOnConflict, GachaRecordSaver, JsonProperties,
};
use crate::error::{AppError, ErrorDetails};

const MANUAL_INSERT_PULL_COUNT_MAX: u32 = 5000;

const CATEGORY_CHARACTER: &str = "Character";
const CATEGORY_WEAPON: &str = "Weapon";
const CATEGORY_BANGBOO: &str = "Bangboo";

const MANUAL_INSERT_CATEGORIES_CHARACTER: [&str; 1] = [CATEGORY_CHARACTER];
const MANUAL_INSERT_CATEGORIES_WEAPON: [&str; 1] = [CATEGORY_WEAPON];
const MANUAL_INSERT_CATEGORIES_BANGBOO: [&str; 1] = [CATEGORY_BANGBOO];
const MANUAL_INSERT_CATEGORIES_CHARACTER_OR_WEAPON: [&str; 2] =
  [CATEGORY_CHARACTER, CATEGORY_WEAPON];

#[derive(Debug, Snafu)]
#[snafu(visibility)]
pub enum ManualInsertGachaRecordsError {
  #[snafu(display("Unsupported business: {business:?}"))]
  UnsupportedBusiness { business: AccountBusiness },

  #[snafu(display("Unsupported gacha type: {gacha_type} for business: {business:?}"))]
  UnsupportedGachaType {
    business: AccountBusiness,
    gacha_type: u32,
  },

  #[snafu(display("Invalid pull count: {pull_count}"))]
  InvalidPullCount { pull_count: u32 },

  #[snafu(display("Invalid end time format: {value}, cause: {cause}"))]
  InvalidEndTime { value: String, cause: String },

  #[snafu(display("Missing metadata locale: {locale} ({business:?})"))]
  MissingMetadataLocale {
    business: AccountBusiness,
    locale: String,
  },

  #[snafu(display("Character not found in metadata: {name} ({business:?}, {locale})"))]
  CharacterNotFound {
    business: AccountBusiness,
    locale: String,
    name: String,
  },

  #[snafu(display(
    "The character is not a supported 5-star entry: {name} ({business:?}, {locale})"
  ))]
  InvalidCharacter {
    business: AccountBusiness,
    locale: String,
    name: String,
  },

  #[snafu(display("Missing default item metadata entry: id={item_id} ({business:?}, {locale})"))]
  MissingDefaultMetadataEntry {
    business: AccountBusiness,
    locale: String,
    item_id: u32,
  },

  #[snafu(display("Database error: {cause}"))]
  Sqlx { cause: String },
}

impl ErrorDetails for ManualInsertGachaRecordsError {
  fn name(&self) -> &'static str {
    stringify!(ManualInsertGachaRecordsError)
  }

  fn details(&self) -> Option<serde_json::Value> {
    use serde_json::json;

    Some(match self {
      Self::UnsupportedBusiness { business } => json!({
        "kind": stringify!(UnsupportedBusiness),
        "business": business,
      }),
      Self::UnsupportedGachaType {
        business,
        gacha_type,
      } => json!({
        "kind": stringify!(UnsupportedGachaType),
        "business": business,
        "gacha_type": gacha_type,
      }),
      Self::InvalidPullCount { pull_count } => json!({
        "kind": stringify!(InvalidPullCount),
        "pull_count": pull_count,
      }),
      Self::InvalidEndTime { value, cause } => json!({
        "kind": stringify!(InvalidEndTime),
        "value": value,
        "cause": cause,
      }),
      Self::MissingMetadataLocale { business, locale } => json!({
        "kind": stringify!(MissingMetadataLocale),
        "business": business,
        "locale": locale,
      }),
      Self::CharacterNotFound {
        business,
        locale,
        name,
      } => json!({
        "kind": stringify!(CharacterNotFound),
        "business": business,
        "locale": locale,
        "name": name,
      }),
      Self::InvalidCharacter {
        business,
        locale,
        name,
      } => json!({
        "kind": stringify!(InvalidCharacter),
        "business": business,
        "locale": locale,
        "name": name,
      }),
      Self::MissingDefaultMetadataEntry {
        business,
        locale,
        item_id,
      } => json!({
        "kind": stringify!(MissingDefaultMetadataEntry),
        "business": business,
        "locale": locale,
        "item_id": item_id,
      }),
      Self::Sqlx { cause } => json!({
        "kind": stringify!(Sqlx),
        "cause": cause,
      }),
    })
  }
}

fn is_supported_manual_insert_gacha_type(business: AccountBusiness, gacha_type: u32) -> bool {
  match business {
    AccountBusiness::GenshinImpact => matches!(gacha_type, 100 | 200 | 301 | 400 | 302 | 500),
    AccountBusiness::HonkaiStarRail => matches!(gacha_type, 1 | 2 | 11 | 12 | 21 | 22),
    AccountBusiness::ZenlessZoneZero => matches!(gacha_type, 1 | 2 | 3 | 5 | 102 | 103),
    AccountBusiness::MiliastraWonderland => false,
  }
}

fn manual_insert_default_item_ids(
  business: AccountBusiness,
) -> Result<(u32, u32), ManualInsertGachaRecordsError> {
  match business {
    AccountBusiness::GenshinImpact => Ok((12305, 11401)),
    AccountBusiness::HonkaiStarRail => Ok((20000, 21000)),
    AccountBusiness::ZenlessZoneZero => Ok((12001, 13001)),
    AccountBusiness::MiliastraWonderland => UnsupportedBusinessSnafu { business }.fail(),
  }
}

fn manual_insert_required_golden_rank(
  business: AccountBusiness,
) -> Result<u8, ManualInsertGachaRecordsError> {
  match business {
    AccountBusiness::GenshinImpact | AccountBusiness::HonkaiStarRail => Ok(5),
    AccountBusiness::ZenlessZoneZero => Ok(4),
    AccountBusiness::MiliastraWonderland => UnsupportedBusinessSnafu { business }.fail(),
  }
}

fn manual_insert_supported_golden_categories(
  business: AccountBusiness,
  gacha_type: u32,
) -> Result<&'static [&'static str], ManualInsertGachaRecordsError> {
  let categories = match business {
    AccountBusiness::GenshinImpact => match gacha_type {
      301 | 400 => &MANUAL_INSERT_CATEGORIES_CHARACTER[..],
      302 => &MANUAL_INSERT_CATEGORIES_WEAPON[..],
      100 | 200 | 500 => &MANUAL_INSERT_CATEGORIES_CHARACTER_OR_WEAPON[..],
      _ => {
        return UnsupportedGachaTypeSnafu {
          business,
          gacha_type,
        }
        .fail();
      }
    },
    AccountBusiness::HonkaiStarRail => match gacha_type {
      11 | 12 => &MANUAL_INSERT_CATEGORIES_CHARACTER[..],
      21 | 22 => &MANUAL_INSERT_CATEGORIES_WEAPON[..],
      1 | 2 => &MANUAL_INSERT_CATEGORIES_CHARACTER_OR_WEAPON[..],
      _ => {
        return UnsupportedGachaTypeSnafu {
          business,
          gacha_type,
        }
        .fail();
      }
    },
    AccountBusiness::ZenlessZoneZero => match gacha_type {
      1 | 102 => &MANUAL_INSERT_CATEGORIES_CHARACTER[..],
      2 | 103 => &MANUAL_INSERT_CATEGORIES_WEAPON[..],
      3 => &MANUAL_INSERT_CATEGORIES_CHARACTER_OR_WEAPON[..],
      5 => &MANUAL_INSERT_CATEGORIES_BANGBOO[..],
      _ => {
        return UnsupportedGachaTypeSnafu {
          business,
          gacha_type,
        }
        .fail();
      }
    },
    AccountBusiness::MiliastraWonderland => {
      return UnsupportedBusinessSnafu { business }.fail();
    }
  };

  Ok(categories)
}

fn manual_insert_entry_name_candidates(name: &str) -> Vec<String> {
  let name = name.trim();
  let mut candidates = vec![name.to_owned()];

  if !name.starts_with('「') && !name.ends_with('」') {
    candidates.push(format!("「{name}」"));
  }

  if let Some(unwrapped) = name.strip_prefix('「').and_then(|s| s.strip_suffix('」'))
    && !unwrapped.is_empty()
  {
    candidates.push(unwrapped.to_owned());
  }

  candidates
}

#[inline]
fn manual_insert_entry_match(
  entry: &Entry<'_>,
  allowed_categories: &[&'static str],
  golden_rank: u8,
) -> bool {
  entry.rank_type == golden_rank
    && allowed_categories
      .iter()
      .any(|category| *category == entry.category)
}

fn manual_insert_find_entry_in_locale<'a, 'name: 'a>(
  locale: &'a dyn hg_metadata::MetadataLocale,
  name: &'name str,
  allowed_categories: &[&'static str],
  golden_rank: u8,
) -> Option<Entry<'a>> {
  locale
    .entry_from_name(name)?
    .into_iter()
    .find(|entry| manual_insert_entry_match(entry, allowed_categories, golden_rank))
}

fn resolve_manual_insert_locale(custom_locale: Option<String>) -> String {
  custom_locale
    .filter(|locale| !locale.trim().is_empty())
    .unwrap_or_else(|| String::from("en-us"))
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualInsertUpBanner {
  #[serde(with = "rfc3339")]
  pub start_time: OffsetDateTime,
  #[serde(with = "rfc3339")]
  pub end_time: OffsetDateTime,
  pub version: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualInsertEntryOption {
  pub item_id: u32,
  pub name: String,
  pub item_type: String,
  pub rank_type: u8,
  pub up_banners: Vec<ManualInsertUpBanner>,
}

#[tauri::command]
#[tracing::instrument(skip(metadata))]
pub async fn business_manual_insert_gacha_entry_options(
  metadata: TauriMetadataState<'_>,
  business: AccountBusiness,
  gacha_type: u32,
  custom_locale: Option<String>,
) -> Result<Vec<ManualInsertEntryOption>, AppError<ManualInsertGachaRecordsError>> {
  if !is_supported_manual_insert_gacha_type(business, gacha_type) {
    return Err(AppError::from(
      ManualInsertGachaRecordsError::UnsupportedGachaType {
        business,
        gacha_type,
      },
    ));
  }

  let locale = resolve_manual_insert_locale(custom_locale);

  let metadata = { &*metadata.read().await };
  let metadata_locale =
    metadata
      .locale(business as _, &locale)
      .context(MissingMetadataLocaleSnafu {
        business,
        locale: locale.clone(),
      })?;

  let allowed_categories = manual_insert_supported_golden_categories(business, gacha_type)?;
  let golden_rank = manual_insert_required_golden_rank(business)?;
  let banners = metadata
    .banners(business as _, gacha_type)
    .unwrap_or_default();
  let mut entries = metadata_locale
    .entries()
    .into_values()
    .filter(|entry| manual_insert_entry_match(entry, allowed_categories, golden_rank))
    .map(|entry| {
      let mut up_banners = banners
        .iter()
        .filter(|banner| banner.is_up_golden(entry.item_id))
        .map(|banner| ManualInsertUpBanner {
          start_time: *banner.start_time(),
          end_time: *banner.end_time(),
          version: banner.version().map(str::to_owned),
        })
        .collect::<Vec<_>>();

      up_banners.sort_by(|a, b| b.start_time.cmp(&a.start_time));

      ManualInsertEntryOption {
        item_id: entry.item_id,
        name: entry.item_name.to_owned(),
        item_type: entry.category_name.to_owned(),
        rank_type: entry.rank_type,
        up_banners,
      }
    })
    .collect::<Vec<_>>();

  entries.sort_by(|a, b| {
    a.item_type
      .cmp(&b.item_type)
      .then_with(|| a.name.cmp(&b.name))
      .then_with(|| a.item_id.cmp(&b.item_id))
  });

  Ok(entries)
}

#[tauri::command]
#[tracing::instrument(skip(database, metadata))]
pub async fn business_manual_insert_gacha_records(
  database: TauriDatabaseState<'_>,
  metadata: TauriMetadataState<'_>,
  business: AccountBusiness,
  uid: u32,
  gacha_type: u32,
  five_star_name: String,
  pull_count: u32,
  end_time: String,
  custom_locale: Option<String>,
) -> Result<u64, AppError<ManualInsertGachaRecordsError>> {
  if !is_supported_manual_insert_gacha_type(business, gacha_type) {
    return Err(AppError::from(
      ManualInsertGachaRecordsError::UnsupportedGachaType {
        business,
        gacha_type,
      },
    ));
  }

  if pull_count == 0 || pull_count > MANUAL_INSERT_PULL_COUNT_MAX {
    return Err(AppError::from(
      ManualInsertGachaRecordsError::InvalidPullCount { pull_count },
    ));
  }

  let end_time = OffsetDateTime::parse(&end_time, &Rfc3339).map_err(|cause| {
    AppError::from(ManualInsertGachaRecordsError::InvalidEndTime {
      value: end_time,
      cause: cause.to_string(),
    })
  })?;

  let locale = resolve_manual_insert_locale(custom_locale);

  let metadata = { &*metadata.read().await };
  let metadata_locale =
    metadata
      .locale(business as _, &locale)
      .context(MissingMetadataLocaleSnafu {
        business,
        locale: locale.clone(),
      })?;

  let metadata_locale_name = metadata_locale.lang().to_owned();
  let golden_rank = manual_insert_required_golden_rank(business)?;
  let allowed_categories = manual_insert_supported_golden_categories(business, gacha_type)?;
  let candidate_names = manual_insert_entry_name_candidates(&five_star_name);

  let mut name_exists = false;
  let mut five_star_entry = None;
  for candidate in &candidate_names {
    if let Some(entry) = manual_insert_find_entry_in_locale(
      metadata_locale,
      candidate,
      allowed_categories,
      golden_rank,
    ) {
      five_star_entry = Some(entry);
      break;
    }

    if let Some(locales) = metadata.locales(business as _) {
      for locale in locales {
        if locale.entry_from_name_first(candidate).is_some() {
          name_exists = true;
        }

        if let Some(entry) = manual_insert_find_entry_in_locale(
          locale.as_ref(),
          candidate,
          allowed_categories,
          golden_rank,
        )
        .and_then(|entry| metadata_locale.entry_from_id(entry.item_id))
        {
          five_star_entry = Some(entry);
          break;
        }
      }
    }

    if five_star_entry.is_some() {
      break;
    }
  }

  let name_exists = name_exists
    || candidate_names
      .iter()
      .any(|candidate| metadata_locale.entry_from_name_first(candidate).is_some());

  let five_star_entry = five_star_entry.ok_or_else(|| {
    if name_exists {
      AppError::from(ManualInsertGachaRecordsError::InvalidCharacter {
        business,
        locale: metadata_locale_name.clone(),
        name: five_star_name.clone(),
      })
    } else {
      AppError::from(ManualInsertGachaRecordsError::CharacterNotFound {
        business,
        locale: metadata_locale_name.clone(),
        name: five_star_name.clone(),
      })
    }
  })?;

  let (blue_item_id, purple_item_id) = manual_insert_default_item_ids(business)?;
  let blue_entry =
    metadata_locale
      .entry_from_id(blue_item_id)
      .context(MissingDefaultMetadataEntrySnafu {
        business,
        locale: metadata_locale_name.clone(),
        item_id: blue_item_id,
      })?;
  let purple_entry =
    metadata_locale
      .entry_from_id(purple_item_id)
      .context(MissingDefaultMetadataEntrySnafu {
        business,
        locale: metadata_locale_name.clone(),
        item_id: purple_item_id,
      })?;

  let start_time = end_time - Duration::seconds((pull_count - 1) as i64);
  let mut records = Vec::with_capacity(pull_count as usize);
  let manual_insert_properties = Some(
    [(
      GachaRecord::KEY_MANUAL_INSERT.to_owned(),
      JsonValue::Bool(true),
    )]
    .into_iter()
    .collect::<JsonProperties>(),
  );

  for index in 0..pull_count {
    let pull_number = index + 1;
    let time = start_time + Duration::seconds(index as i64);

    let (item_name, item_type, item_id, rank_type) = if pull_number == pull_count {
      (
        five_star_entry.item_name.to_owned(),
        five_star_entry.category_name.to_owned(),
        five_star_entry.item_id,
        five_star_entry.rank_type as u32,
      )
    } else if pull_number % 8 == 0 {
      (
        purple_entry.item_name.to_owned(),
        purple_entry.category_name.to_owned(),
        purple_entry.item_id,
        purple_entry.rank_type as u32,
      )
    } else {
      (
        blue_entry.item_name.to_owned(),
        blue_entry.category_name.to_owned(),
        blue_entry.item_id,
        blue_entry.rank_type as u32,
      )
    };

    let unix_ts = time.unix_timestamp();
    let id = format!("{unix_ts}{index:09}");

    records.push(GachaRecord {
      business,
      uid,
      id,
      gacha_type,
      gacha_id: match business {
        AccountBusiness::HonkaiStarRail | AccountBusiness::ZenlessZoneZero => {
          Some(unix_ts.max(0) as u32)
        }
        _ => None,
      },
      rank_type,
      count: 1,
      lang: metadata_locale_name.clone(),
      time,
      item_name,
      item_type,
      item_id,
      properties: manual_insert_properties.clone(),
    });
  }

  GachaRecordSaver::new(
    &records,
    GachaRecordSaveOnConflict::Nothing,
    Option::<fn(u64)>::None,
  )
  .save(&database)
  .await
  .map_err(|cause| {
    AppError::from(ManualInsertGachaRecordsError::Sqlx {
      cause: cause.to_string(),
    })
  })
}
