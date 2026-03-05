use std::collections::{HashMap, hash_map};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{Emitter, WebviewWindow};
use time::format_description::FormatItem;
use time::format_description::well_known::Rfc3339;
use time::macros::format_description;
use time::serde::rfc3339;
use time::{Duration, OffsetDateTime};
use tokio::sync::mpsc;

use crate::consts;
use crate::database::{
  DatabaseState, GachaRecordQuestioner, GachaRecordQuestionerAdditions, GachaRecordSaveOnConflict,
};
use crate::error::declare_error_kinds;
use crate::error::{BoxDynErrorDetails, Error};
use crate::models::{Business, BusinessRegion, GachaRecord};

mod data_folder_locator;
mod disk_cache;
mod gacha_convert;
mod gacha_fetcher;
mod gacha_metadata;
mod gacha_prettied;
mod gacha_url;

pub use data_folder_locator::*;
pub use gacha_convert::*;
pub use gacha_fetcher::*;
pub use gacha_metadata::*;
pub use gacha_prettied::*;
pub use gacha_url::*;

pub const GACHA_TIME_FORMAT: &[FormatItem<'_>] =
  format_description!("[year]-[month]-[day] [hour]:[minute]:[second]");

time::serde::format_description!(pub gacha_time_format, PrimitiveDateTime, GACHA_TIME_FORMAT);

declare_error_kinds! {
  #[derive(Debug, thiserror::Error)]
  ManualInsertGachaRecordsError {
    #[error("Unsupported business: {business}")]
    UnsupportedBusiness { business: Business },

    #[error("Unsupported gacha type: {gacha_type} for business: {business}")]
    UnsupportedGachaType { business: Business, gacha_type: u32 },

    #[error("Invalid pull count: {pull_count}")]
    InvalidPullCount { pull_count: u32 },

    #[error("Invalid end time format: {value}, cause: {cause}")]
    InvalidEndTime { value: String, cause: String },

    #[error("Missing metadata locale: {locale} ({business})")]
    MissingMetadataLocale { business: Business, locale: String },

    #[error("Character not found in metadata: {name} ({business}, {locale})")]
    CharacterNotFound {
      business: Business,
      locale: String,
      name: String
    },

    #[error("The character is not a supported 5-star entry: {name} ({business}, {locale})")]
    InvalidCharacter {
      business: Business,
      locale: String,
      name: String
    },

    #[error("Missing default item metadata entry: id={item_id} ({business}, {locale})")]
    MissingDefaultMetadataEntry {
      business: Business,
      locale: String,
      item_id: u32
    },

    #[error("Database error: {cause}")]
    Sqlx { cause: String },
  }
}

const MANUAL_INSERT_PULL_COUNT_MAX: u32 = 5000;

fn is_supported_manual_insert_gacha_type(business: Business, gacha_type: u32) -> bool {
  match business {
    Business::GenshinImpact => matches!(gacha_type, 100 | 200 | 301 | 400 | 302 | 500),
    Business::HonkaiStarRail => matches!(gacha_type, 1 | 2 | 11 | 12 | 21 | 22),
    Business::ZenlessZoneZero => matches!(gacha_type, 1 | 2 | 3 | 5 | 102 | 103),
    Business::MiliastraWonderland => false,
  }
}

fn manual_insert_default_item_ids(
  business: Business,
) -> Result<(u32, u32), ManualInsertGachaRecordsError> {
  match business {
    Business::GenshinImpact => Ok((12305, 11401)),
    Business::HonkaiStarRail => Ok((20000, 21000)),
    // 2-star + 3-star W-Engine
    Business::ZenlessZoneZero => Ok((12001, 13001)),
    Business::MiliastraWonderland => {
      Err(ManualInsertGachaRecordsErrorKind::UnsupportedBusiness { business })?
    }
  }
}

fn manual_insert_required_golden_rank(
  business: Business,
) -> Result<u8, ManualInsertGachaRecordsError> {
  match business {
    Business::GenshinImpact | Business::HonkaiStarRail => Ok(5),
    Business::ZenlessZoneZero => Ok(4),
    Business::MiliastraWonderland => {
      Err(ManualInsertGachaRecordsErrorKind::UnsupportedBusiness { business })?
    }
  }
}

const MANUAL_INSERT_CATEGORIES_CHARACTER: [&str; 1] = [GachaMetadata::CATEGORY_CHARACTER];
const MANUAL_INSERT_CATEGORIES_WEAPON: [&str; 1] = [GachaMetadata::CATEGORY_WEAPON];
const MANUAL_INSERT_CATEGORIES_BANGBOO: [&str; 1] = [GachaMetadata::CATEGORY_BANGBOO];
const MANUAL_INSERT_CATEGORIES_CHARACTER_OR_WEAPON: [&str; 2] = [
  GachaMetadata::CATEGORY_CHARACTER,
  GachaMetadata::CATEGORY_WEAPON,
];

fn manual_insert_supported_golden_categories(
  business: Business,
  gacha_type: u32,
) -> Result<&'static [&'static str], ManualInsertGachaRecordsError> {
  let categories = match business {
    Business::GenshinImpact => match gacha_type {
      301 | 400 => &MANUAL_INSERT_CATEGORIES_CHARACTER[..],
      302 => &MANUAL_INSERT_CATEGORIES_WEAPON[..],
      100 | 200 | 500 => &MANUAL_INSERT_CATEGORIES_CHARACTER_OR_WEAPON[..],
      _ => {
        return Err(
          ManualInsertGachaRecordsErrorKind::UnsupportedGachaType {
            business,
            gacha_type,
          }
          .into(),
        );
      }
    },
    Business::HonkaiStarRail => match gacha_type {
      11 | 12 => &MANUAL_INSERT_CATEGORIES_CHARACTER[..],
      21 | 22 => &MANUAL_INSERT_CATEGORIES_WEAPON[..],
      1 | 2 => &MANUAL_INSERT_CATEGORIES_CHARACTER_OR_WEAPON[..],
      _ => {
        return Err(
          ManualInsertGachaRecordsErrorKind::UnsupportedGachaType {
            business,
            gacha_type,
          }
          .into(),
        );
      }
    },
    Business::ZenlessZoneZero => match gacha_type {
      1 | 102 => &MANUAL_INSERT_CATEGORIES_CHARACTER[..],
      2 | 103 => &MANUAL_INSERT_CATEGORIES_WEAPON[..],
      3 => &MANUAL_INSERT_CATEGORIES_CHARACTER_OR_WEAPON[..],
      5 => &MANUAL_INSERT_CATEGORIES_BANGBOO[..],
      _ => {
        return Err(
          ManualInsertGachaRecordsErrorKind::UnsupportedGachaType {
            business,
            gacha_type,
          }
          .into(),
        );
      }
    },
    Business::MiliastraWonderland => {
      return Err(ManualInsertGachaRecordsErrorKind::UnsupportedBusiness { business }.into());
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
  entry: &GachaMetadataEntryRef<'_>,
  allowed_categories: &[&'static str],
  golden_rank: u8,
) -> bool {
  entry.rank == golden_rank
    && allowed_categories
      .iter()
      .any(|category| *category == entry.category)
}

fn manual_insert_find_entry_in_locale<'a, 'name: 'a>(
  locale: &'a GachaMetadataLocale,
  name: &'name str,
  allowed_categories: &[&'static str],
  golden_rank: u8,
) -> Option<GachaMetadataEntryRef<'a>> {
  locale
    .entry_from_name(name)?
    .into_iter()
    .find(|entry| manual_insert_entry_match(entry, allowed_categories, golden_rank))
}

// region: Tauri plugin

#[tauri::command]
#[tracing::instrument(skip_all)]
pub async fn business_locate_data_folder(
  business: Business,
  region: BusinessRegion,
  factory: DataFolderLocatorFactory,
) -> Result<DataFolder, DataFolderError> {
  factory.locate_data_folder(business, region).await
}

#[tauri::command]
#[tracing::instrument(skip_all)]
pub async fn business_from_webcaches_gacha_url(
  business: Business,
  region: BusinessRegion,
  data_folder: PathBuf,
  expected_uid: u32,
) -> Result<GachaUrl, GachaUrlError> {
  GachaUrl::from_webcaches(business, region, &data_folder, expected_uid).await
}

#[tauri::command]
#[tracing::instrument(skip_all)]
pub async fn business_from_dirty_gacha_url(
  business: Business,
  region: BusinessRegion,
  dirty_url: String,
  expected_uid: u32,
) -> Result<GachaUrl, GachaUrlError> {
  GachaUrl::from_dirty(business, region, dirty_url, expected_uid).await
}

#[derive(Copy, Clone, Debug, Deserialize)]
pub enum GachaRecordSaveToDatabase {
  No,
  Yes,
  FullUpdate,
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
#[tracing::instrument(skip_all)]
pub async fn business_create_gacha_records_fetcher(
  window: WebviewWindow,
  database: DatabaseState<'_>,
  business: Business,
  region: BusinessRegion,
  uid: u32,
  gacha_url: String,
  mut gacha_type_and_last_end_id_mappings: Vec<(u32, Option<String>)>,
  event_channel: Option<String>,
  save_to_database: Option<GachaRecordSaveToDatabase>,
  save_on_conflict: Option<GachaRecordSaveOnConflict>,
) -> Result<i64, BoxDynErrorDetails> {
  let save_to_database = save_to_database.unwrap_or(GachaRecordSaveToDatabase::No);
  let save_on_conflict = save_on_conflict.unwrap_or(GachaRecordSaveOnConflict::Nothing);

  // The last_end_id value is discarded on full update
  if matches!(save_to_database, GachaRecordSaveToDatabase::FullUpdate) {
    for (_, last_end_id) in gacha_type_and_last_end_id_mappings.iter_mut() {
      last_end_id.take();
    }
  }

  let records = create_gacha_records_fetcher(
    business,
    region,
    uid,
    gacha_url,
    gacha_type_and_last_end_id_mappings,
    window,
    event_channel,
  )
  .await?
  .unwrap_or(Vec::new());

  if records.is_empty() {
    return Ok(0);
  }

  match save_to_database {
    GachaRecordSaveToDatabase::No => Ok(0),
    GachaRecordSaveToDatabase::Yes => {
      let changes =
        GachaRecordQuestioner::create_gacha_records(&database, records, save_on_conflict, None)
          .await
          .map_err(Error::boxed)? as i64;

      Ok(changes)
    }
    GachaRecordSaveToDatabase::FullUpdate => {
      let groups: HashMap<u32, Vec<GachaRecord>> =
        records.into_iter().fold(HashMap::new(), |mut acc, record| {
          match acc.entry(record.gacha_type) {
            hash_map::Entry::Occupied(mut o) => {
              o.get_mut().push(record);
            }
            hash_map::Entry::Vacant(o) => {
              o.insert(vec![record]);
            }
          }
          acc
        });

      let mut deleted: i64 = 0;
      let mut created: i64 = 0;

      for (gacha_type, records) in groups {
        if records.is_empty() {
          continue;
        }

        let oldest_end_id = records.last().map(|record| record.id.as_str()).unwrap();

        deleted += GachaRecordQuestioner::delete_gacha_records_by_newer_than_end_id(
          &database,
          business,
          uid,
          gacha_type,
          oldest_end_id,
        )
        .await
        .map_err(Error::boxed)? as i64;

        created +=
          GachaRecordQuestioner::create_gacha_records(&database, records, save_on_conflict, None)
            .await
            .map_err(Error::boxed)? as i64;
      }

      let changes = created - deleted;

      Ok(changes)
    }
  }
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
#[tracing::instrument(skip_all)]
pub async fn business_manual_insert_gacha_entry_options(
  business: Business,
  gacha_type: u32,
  custom_locale: Option<String>,
) -> Result<Vec<ManualInsertEntryOption>, ManualInsertGachaRecordsError> {
  if !is_supported_manual_insert_gacha_type(business, gacha_type) {
    return Err(ManualInsertGachaRecordsErrorKind::UnsupportedGachaType {
      business,
      gacha_type,
    })?;
  }

  let locale = custom_locale
    .or_else(|| consts::LOCALE.value.clone())
    .unwrap_or_else(|| String::from("en-us"));

  let metadata = GachaMetadata::current();
  let metadata_locale = metadata.locale(business, &locale).ok_or_else(|| {
    ManualInsertGachaRecordsErrorKind::MissingMetadataLocale {
      business,
      locale: locale.clone(),
    }
  })?;

  let allowed_categories = manual_insert_supported_golden_categories(business, gacha_type)?;
  let golden_rank = manual_insert_required_golden_rank(business)?;
  let banners = metadata.banners(business, gacha_type).unwrap_or(&[]);

  let mut entries = metadata_locale
    .entries()
    .filter(|entry| manual_insert_entry_match(entry, allowed_categories, golden_rank))
    .map(|entry| {
      let mut up_banners = banners
        .iter()
        .filter(|banner| banner.in_up_golden(entry.id))
        .map(|banner| ManualInsertUpBanner {
          start_time: banner.start_time,
          end_time: banner.end_time,
          version: banner.version.as_ref().map(ToString::to_string),
        })
        .collect::<Vec<_>>();

      up_banners.sort_by(|a, b| b.start_time.cmp(&a.start_time));

      ManualInsertEntryOption {
        item_id: entry.id,
        name: entry.name.to_owned(),
        item_type: entry.category_name.to_owned(),
        rank_type: entry.rank,
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
#[tracing::instrument(skip_all)]
pub async fn business_manual_insert_gacha_records(
  database: DatabaseState<'_>,
  business: Business,
  uid: u32,
  gacha_type: u32,
  five_star_name: String,
  pull_count: u32,
  end_time: String,
  custom_locale: Option<String>,
) -> Result<u64, ManualInsertGachaRecordsError> {
  if !is_supported_manual_insert_gacha_type(business, gacha_type) {
    return Err(ManualInsertGachaRecordsErrorKind::UnsupportedGachaType {
      business,
      gacha_type,
    })?;
  }

  if pull_count == 0 || pull_count > MANUAL_INSERT_PULL_COUNT_MAX {
    return Err(ManualInsertGachaRecordsErrorKind::InvalidPullCount { pull_count })?;
  }

  let end_time = OffsetDateTime::parse(&end_time, &Rfc3339).map_err(|cause| {
    ManualInsertGachaRecordsErrorKind::InvalidEndTime {
      value: end_time,
      cause: cause.to_string(),
    }
  })?;

  let locale = custom_locale
    .or_else(|| consts::LOCALE.value.clone())
    .unwrap_or_else(|| String::from("en-us"));

  let metadata = GachaMetadata::current();
  let metadata_locale = metadata.locale(business, &locale).ok_or_else(|| {
    ManualInsertGachaRecordsErrorKind::MissingMetadataLocale {
      business,
      locale: locale.clone(),
    }
  })?;

  let metadata_locale_name = metadata_locale.locale.clone();
  let golden_rank = manual_insert_required_golden_rank(business)?;
  let allowed_categories = manual_insert_supported_golden_categories(business, gacha_type)?;
  let candidate_names = manual_insert_entry_name_candidates(&five_star_name);
  let business_locales = metadata.metadata.get(&business).map(|data| &data.locales);

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

    if let Some(locales) = business_locales {
      for locale in locales.values() {
        if locale.entry_from_name_first(candidate).is_some() {
          name_exists = true;
        }

        if let Some(entry) =
          manual_insert_find_entry_in_locale(locale, candidate, allowed_categories, golden_rank)
            .and_then(|entry| metadata_locale.entry_from_id(entry.id))
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
    let kind = if name_exists {
      ManualInsertGachaRecordsErrorKind::InvalidCharacter {
        business,
        locale: metadata_locale_name.clone(),
        name: five_star_name.clone(),
      }
    } else {
      ManualInsertGachaRecordsErrorKind::CharacterNotFound {
        business,
        locale: metadata_locale_name.clone(),
        name: five_star_name.clone(),
      }
    };
    ManualInsertGachaRecordsError::from(kind)
  })?;

  let (blue_item_id, purple_item_id) = manual_insert_default_item_ids(business)?;
  let blue_entry = metadata_locale.entry_from_id(blue_item_id).ok_or_else(|| {
    ManualInsertGachaRecordsErrorKind::MissingDefaultMetadataEntry {
      business,
      locale: metadata_locale_name.clone(),
      item_id: blue_item_id,
    }
  })?;
  let purple_entry = metadata_locale
    .entry_from_id(purple_item_id)
    .ok_or_else(
      || ManualInsertGachaRecordsErrorKind::MissingDefaultMetadataEntry {
        business,
        locale: metadata_locale_name.clone(),
        item_id: purple_item_id,
      },
    )?;

  let start_time = end_time - Duration::seconds((pull_count - 1) as i64);
  let mut records = Vec::with_capacity(pull_count as usize);

  for index in 0..pull_count {
    let pull_number = index + 1;
    let time = start_time + Duration::seconds(index as i64);

    let (name, item_type, item_id, rank_type) = if pull_number == pull_count {
      (
        five_star_entry.name.to_owned(),
        five_star_entry.category_name.to_owned(),
        five_star_entry.id,
        five_star_entry.rank as u32,
      )
    } else if pull_number % 8 == 0 {
      (
        purple_entry.name.to_owned(),
        purple_entry.category_name.to_owned(),
        purple_entry.id,
        purple_entry.rank as u32,
      )
    } else {
      (
        blue_entry.name.to_owned(),
        blue_entry.category_name.to_owned(),
        blue_entry.id,
        blue_entry.rank as u32,
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
        Business::HonkaiStarRail | Business::ZenlessZoneZero => Some(unix_ts.max(0) as u32),
        _ => None,
      },
      rank_type,
      count: 1,
      lang: metadata_locale_name.clone(),
      time,
      name,
      item_type,
      item_id,
      properties: None,
    });
  }

  GachaRecordQuestioner::create_gacha_records(
    database.as_ref(),
    records,
    GachaRecordSaveOnConflict::Nothing,
    None,
  )
  .await
  .map_err(|cause| {
    ManualInsertGachaRecordsErrorKind::Sqlx {
      cause: cause.to_string(),
    }
    .into()
  })
}

#[tauri::command]
#[tracing::instrument(skip_all)]
pub async fn business_import_gacha_records(
  window: WebviewWindow,
  database: DatabaseState<'_>,
  input: PathBuf,
  importer: GachaRecordsImporter,
  save_on_conflict: Option<GachaRecordSaveOnConflict>,
  progress_channel: Option<String>,
) -> Result<u64, BoxDynErrorDetails> {
  let records = importer.import(GachaMetadata::current(), input)?;

  // Progress reporting
  let (progress_reporter, progress_task) = if let Some(event_channel) = progress_channel {
    let (reporter, mut receiver) = mpsc::channel(1);
    let task = tokio::spawn(async move {
      while let Some(progress) = receiver.recv().await {
        window.emit(&event_channel, &progress).unwrap(); // FIXME: emit SAFETY?
      }
    });

    (Some(reporter), Some(task))
  } else {
    (None, None)
  };

  let changes = GachaRecordQuestioner::create_gacha_records(
    database.as_ref(),
    records,
    save_on_conflict.unwrap_or(GachaRecordSaveOnConflict::Nothing),
    progress_reporter,
  )
  .await
  .map_err(Error::boxed)?;

  // Wait for the progress task to finish
  if let Some(progress_task) = progress_task {
    progress_task.await.unwrap(); // FIXME: SAFETY?
  }

  Ok(changes)
}

#[tauri::command]
#[tracing::instrument(skip_all)]
pub async fn business_export_gacha_records(
  _window: WebviewWindow,
  database: DatabaseState<'_>,
  output: PathBuf,
  exporter: GachaRecordsExporter,
) -> Result<PathBuf, BoxDynErrorDetails> {
  // TODO: Progress reporting

  let records = match &exporter {
    GachaRecordsExporter::LegacyUigf(writer) => {
      GachaRecordQuestioner::find_gacha_records_by_business_and_uid(
        database.as_ref(),
        Business::GenshinImpact,
        writer.account_uid,
      )
      .await
      .map_err(Error::boxed)?
    }
    GachaRecordsExporter::Uigf(writer) => {
      let mut records = Vec::new();

      for account_uid in writer.accounts.keys() {
        let account_records = GachaRecordQuestioner::find_gacha_records_by_businesses_or_uid(
          database.as_ref(),
          writer.businesses.as_ref(),
          *account_uid,
        )
        .await
        .map_err(Error::boxed)?;

        records.extend(account_records);
      }

      records
    }
    GachaRecordsExporter::Srgf(writer) => {
      GachaRecordQuestioner::find_gacha_records_by_business_and_uid(
        database.as_ref(),
        Business::HonkaiStarRail,
        writer.account_uid,
      )
      .await
      .map_err(Error::boxed)?
    }
    GachaRecordsExporter::Csv(writer) => {
      GachaRecordQuestioner::find_gacha_records_by_business_and_uid(
        database.as_ref(),
        writer.business,
        writer.account_uid,
      )
      .await
      .map_err(Error::boxed)?
    }
  };

  exporter.export(GachaMetadata::current(), records, output)
}

#[tauri::command]
#[tracing::instrument(skip_all)]
pub async fn business_find_and_pretty_gacha_records(
  database: DatabaseState<'_>,
  business: Business,
  uid: u32,
  custom_locale: Option<String>,
) -> Result<PrettiedGachaRecords, BoxDynErrorDetails> {
  let records =
    GachaRecordQuestioner::find_gacha_records_by_business_and_uid(database.as_ref(), business, uid)
      .await
      .map_err(Error::boxed)?;

  let prettied = PrettiedGachaRecords::pretty(
    GachaMetadata::current(),
    business,
    uid,
    &records[..],
    custom_locale.as_deref(),
  );
  // .map_err(Error::boxed)?;

  Ok(prettied)
}

#[tauri::command]
#[tracing::instrument(skip_all)]
pub async fn business_gacha_metadata_is_updating() -> bool {
  GachaMetadata::is_updating()
}

#[tauri::command]
#[tracing::instrument(skip_all)]
pub async fn business_gacha_metadata_update() -> Result<GachaMetadataUpdatedKind, String> {
  GachaMetadata::update().await.map_err(|e| e.to_string())
}

#[tauri::command]
#[tracing::instrument(skip_all)]
pub async fn business_gacha_metadata_item_name_from_id(
  business: Business,
  item_id: u32,
  locale: String,
) -> Option<String> {
  GachaMetadata::current()
    .locale(business, locale)?
    .entry_from_id(item_id)
    .map(|entry| entry.name.to_owned())
}

// endregion
