import React, { forwardRef, useCallback, useEffect, useImperativeHandle, useMemo, useRef, useState } from 'react'
import { SubmitHandler, useForm, useWatch } from 'react-hook-form'
import { Button, Dialog, DialogBody, DialogContent, DialogSurface, DialogTitle, Field, Input, Select, makeStyles, tokens } from '@fluentui/react-components'
import { produce } from 'immer'
import {
  ManualInsertEntryOption,
  ManualInsertGachaEntryOptionsArgs,
  ManualInsertGachaRecordsArgs,
  manualInsertGachaEntryOptions,
  manualInsertGachaRecords,
} from '@/api/commands/business'
import errorTranslation from '@/api/errorTranslation'
import { useSelectedAccountSuspenseQueryData, useUpdateAccountPropertiesMutation } from '@/api/queries/accounts'
import { invalidateFirstGachaRecordQuery, invalidatePrettizedGachaRecordsQuery } from '@/api/queries/business'
import Locale from '@/components/Locale'
import useI18n from '@/hooks/useI18n'
import useNotifier from '@/hooks/useNotifier'
import { Business, KeyofBusinesses } from '@/interfaces/Business'
import dayjs from '@/utilities/dayjs'

const useStyles = makeStyles({
  form: {
    display: 'flex',
    flexDirection: 'column',
    rowGap: tokens.spacingVerticalS,
    minWidth: '24rem',
  },
  hint: {
    color: tokens.colorNeutralForeground3,
    fontSize: tokens.fontSizeBase200,
    marginTop: tokens.spacingVerticalXXS,
  },
  actions: {
    display: 'flex',
    flexDirection: 'row',
    justifyContent: 'flex-end',
    columnGap: tokens.spacingHorizontalS,
  },
})

const ManualInsertGachaTypeOptions: Record<number, Array<{
  value: number
  categoryKey: string
  suffix?: string
}>> = {
  0: [
    { value: 100, categoryKey: 'Beginner' },
    { value: 200, categoryKey: 'Permanent' },
    { value: 301, categoryKey: 'Character', suffix: '1' },
    { value: 400, categoryKey: 'Character', suffix: '2' },
    { value: 302, categoryKey: 'Weapon' },
    { value: 500, categoryKey: 'Chronicled' },
  ],
  1: [
    { value: 2, categoryKey: 'Beginner' },
    { value: 1, categoryKey: 'Permanent' },
    { value: 11, categoryKey: 'Character', suffix: '1' },
    { value: 12, categoryKey: 'CollaborationCharacter' },
    { value: 21, categoryKey: 'Weapon', suffix: '1' },
    { value: 22, categoryKey: 'CollaborationWeapon' },
  ],
  2: [
    { value: 3, categoryKey: 'Permanent' },
    { value: 1, categoryKey: 'Character' },
    { value: 102, categoryKey: 'ExclusiveRescreening' },
    { value: 2, categoryKey: 'Weapon' },
    { value: 103, categoryKey: 'WEngineReverberation' },
    { value: 5, categoryKey: 'Bangboo' },
  ],
}

const defaultFormValues = (business: Business) => ({
  gachaType: String(ManualInsertGachaTypeOptions[business]?.[0]?.value ?? ''),
  fiveStarItemId: '',
  pullCount: '1',
  endTime: dayjs().format('YYYY-MM-DDTHH:mm:ss'),
})

type FormData = {
  gachaType: string
  fiveStarItemId: string
  pullCount: string
  endTime: string
}

type ManualInsertGachaType = ManualInsertGachaRecordsArgs<Business>['gachaType']

function isManualInsertGachaType (
  value: number,
  options: Array<{ value: number }>,
): value is ManualInsertGachaType {
  return options.some((option) => option.value === value)
}

const ManualInsertDialog = forwardRef<{
  setOpen: React.Dispatch<React.SetStateAction<boolean>>
}, {
  business: Business
  keyofBusinesses: KeyofBusinesses
}>(function ManualInsertDialog (props, ref) {
  const { business, keyofBusinesses } = props
  const styles = useStyles()
  const [open, setOpen] = useState(false)
  const [entryOptions, setEntryOptions] = useState<ManualInsertEntryOption[]>([])
  const [loadingEntryOptions, setLoadingEntryOptions] = useState(false)
  const [selectedUpBannerIndex, setSelectedUpBannerIndex] = useState('')
  const i18n = useI18n()
  const gachaLocale = i18n.constants.gacha
  const notifier = useNotifier()
  const i18nRef = useRef(i18n)
  const notifierRef = useRef(notifier)
  const selectedAccount = useSelectedAccountSuspenseQueryData(keyofBusinesses)
  const updateAccountPropertiesMutation = useUpdateAccountPropertiesMutation()
  const gachaTypeOptions = useMemo(
    () => ManualInsertGachaTypeOptions[business] ?? [],
    [business],
  )

  useImperativeHandle(ref, () => ({ setOpen }))

  const {
    control,
    register,
    getValues,
    handleSubmit,
    reset,
    setError,
    setValue,
    formState: { errors, isValid, isSubmitting },
  } = useForm<FormData>({
    mode: 'onChange',
    defaultValues: defaultFormValues(business),
  })

  const watchedGachaType = useWatch({ control, name: 'gachaType' }) ?? ''
  const watchedItemId = useWatch({ control, name: 'fiveStarItemId' }) ?? ''

  const selectedEntry = useMemo(() => {
    return entryOptions.find((entry) => String(entry.itemId) === watchedItemId) ?? null
  }, [entryOptions, watchedItemId])

  useEffect(() => {
    i18nRef.current = i18n
    notifierRef.current = notifier
  }, [i18n, notifier])

  useEffect(() => {
    reset(defaultFormValues(business))
    setEntryOptions([])
    setSelectedUpBannerIndex('')
  }, [business, reset])

  useEffect(() => {
    const gachaType = Number.parseInt(watchedGachaType, 10)
    if (!Number.isSafeInteger(gachaType) || !isManualInsertGachaType(gachaType, gachaTypeOptions)) {
      setEntryOptions([])
      setSelectedUpBannerIndex('')
      setLoadingEntryOptions(false)
      setValue('fiveStarItemId', '', {
        shouldValidate: true,
      })
      return
    }

    let disposed = false
    setLoadingEntryOptions(true)

    const args: ManualInsertGachaEntryOptionsArgs<Business> = {
      business,
      gachaType,
      customLocale: gachaLocale,
    }

    const loadEntryOptions = async () => {
      try {
        const entries = await manualInsertGachaEntryOptions(args)
        if (disposed) {
          return
        }

        setEntryOptions(entries)
        const currentItemId = getValues('fiveStarItemId')
        const hasSelectedItem = entries.some((entry) => String(entry.itemId) === currentItemId)

        if (!hasSelectedItem) {
          setValue('fiveStarItemId', '', {
            shouldValidate: true,
          })
        }

        setSelectedUpBannerIndex('')
      } catch (error) {
        if (disposed) {
          return
        }

        setEntryOptions([])
        setSelectedUpBannerIndex('')
        setValue('fiveStarItemId', '', {
          shouldValidate: true,
        })
        notifierRef.current.error(
          i18nRef.current.t('Pages.Gacha.LegacyView.Toolbar.Url.ManualInsert.LoadEntryOptionsError', { keyofBusinesses }),
          {
            body: errorTranslation(i18nRef.current, error),
            timeout: notifierRef.current.DefaultTimeouts.error * 2,
            dismissible: true,
          },
        )
      } finally {
        if (!disposed) {
          setLoadingEntryOptions(false)
        }
      }
    }

    loadEntryOptions().catch(() => undefined)

    return () => {
      disposed = true
    }
  }, [business, gachaLocale, gachaTypeOptions, getValues, keyofBusinesses, setValue, watchedGachaType])

  const close = useCallback(() => {
    reset(defaultFormValues(business))
    setEntryOptions([])
    setSelectedUpBannerIndex('')
    setOpen(false)
  }, [business, reset])

  const handleUpBannerSelect = useCallback((event: React.ChangeEvent<HTMLSelectElement>) => {
    const selectedIndex = event.target.value
    setSelectedUpBannerIndex(selectedIndex)

    const parsedIndex = Number.parseInt(selectedIndex, 10)
    if (!selectedEntry || !Number.isSafeInteger(parsedIndex)) {
      return
    }

    const banner = selectedEntry.upBanners[parsedIndex]
    if (!banner) {
      return
    }

    setValue('endTime', dayjs(banner.endTime).format('YYYY-MM-DDTHH:mm:ss'), {
      shouldValidate: true,
      shouldDirty: true,
    })
  }, [selectedEntry, setValue])

  const handleSubmitInner = useCallback<SubmitHandler<FormData>>(async (data) => {
    if (!selectedAccount) {
      return
    }

    const pullCount = Number.parseInt(data.pullCount, 10)
    if (!Number.isSafeInteger(pullCount) || pullCount <= 0) {
      setError('pullCount', {
        message: i18n.t('Pages.Gacha.LegacyView.Toolbar.Url.ManualInsertDialog.PullCount.ValidateMin'),
      })
      return
    }

    const gachaType = Number.parseInt(data.gachaType, 10)
    if (!Number.isSafeInteger(gachaType) || !isManualInsertGachaType(gachaType, gachaTypeOptions)) {
      return
    }

    if (!selectedEntry) {
      setError('fiveStarItemId', {
        message: i18n.t('Pages.Gacha.LegacyView.Toolbar.Url.ManualInsertDialog.FiveStarName.Required'),
      })
      return
    }

    const date = dayjs(data.endTime)
    if (!date.isValid()) {
      setError('endTime', {
        message: i18n.t('Pages.Gacha.LegacyView.Toolbar.Url.ManualInsertDialog.EndTime.Validate'),
      })
      return
    }

    const args: ManualInsertGachaRecordsArgs<Business> = {
      business,
      uid: selectedAccount.uid,
      gachaType,
      fiveStarName: selectedEntry.name,
      pullCount,
      endTime: date.toDate().toISOString(),
      customLocale: gachaLocale,
    }

    const changes = await notifier.promise(
      manualInsertGachaRecords(args),
      {
        loading: {
          title: i18n.t('Pages.Gacha.LegacyView.Toolbar.Url.ManualInsert.Loading', { keyofBusinesses }),
        },
        success: (result) => ({
          title: i18n.t('Pages.Gacha.LegacyView.Toolbar.Url.ManualInsert.Success.Title', { keyofBusinesses }),
          body: i18n.t('Pages.Gacha.LegacyView.Toolbar.Url.ManualInsert.Success.Body', { changes: result }),
          timeout: notifier.DefaultTimeouts.success * 2,
          dismissible: true,
        }),
        error: (error) => ({
          title: i18n.t('Pages.Gacha.LegacyView.Toolbar.Url.ManualInsert.Error', { keyofBusinesses }),
          body: errorTranslation(i18n, error),
          timeout: notifier.DefaultTimeouts.error * 2,
          dismissible: true,
        }),
      },
    )

    if (!changes) {
      close()
      return
    }

    const now = dayjs().toISOString()
    const properties = selectedAccount.properties
      ? produce(selectedAccount.properties, (draft) => {
        draft.lastGachaRecordsUpdated = now
      })
      : { lastGachaRecordsUpdated: now }

    await updateAccountPropertiesMutation.mutateAsync({
      business,
      uid: selectedAccount.uid,
      properties,
    })

    invalidatePrettizedGachaRecordsQuery(selectedAccount.business, selectedAccount.uid, gachaLocale)
    invalidateFirstGachaRecordQuery(selectedAccount.business, selectedAccount.uid)
    close()
  }, [
    business,
    close,
    gachaLocale,
    gachaTypeOptions,
    i18n,
    keyofBusinesses,
    notifier,
    selectedAccount,
    selectedEntry,
    setError,
    updateAccountPropertiesMutation,
  ])

  return (
    <Dialog modalType="alert" open={open}>
      <DialogSurface>
        <DialogBody>
          <Locale
            component={DialogTitle}
            mapping={['Pages.Gacha.LegacyView.Toolbar.Url.ManualInsertDialog.Title']}
          />
          <DialogContent>
            <form className={styles.form} onSubmit={handleSubmit(handleSubmitInner)} noValidate>
              <Field
                size="large"
                label={<Locale mapping={['Pages.Gacha.LegacyView.Toolbar.Url.ManualInsertDialog.GachaType.Label']} />}
                required
              >
                <Select
                  appearance="filled-darker"
                  disabled={isSubmitting}
                  {...register('gachaType', {
                    required: true,
                  })}
                >
                  {gachaTypeOptions.map((option) => {
                    const title = i18n.t(
                      `Business.${keyofBusinesses}.Gacha.Category.${option.categoryKey}`,
                    )
                    return (
                      <option key={option.value} value={option.value}>
                        {option.suffix ? `${title}-${option.suffix}` : title}
                      </option>
                    )
                  })}
                </Select>
              </Field>
              <Field
                size="large"
                validationState={errors.fiveStarItemId ? 'error' : isValid ? 'success' : 'none'}
                validationMessage={errors.fiveStarItemId?.message}
                label={<Locale mapping={['Pages.Gacha.LegacyView.Toolbar.Url.ManualInsertDialog.FiveStarName.Label']} />}
                required
              >
                <Select
                  appearance="filled-darker"
                  disabled={isSubmitting || loadingEntryOptions || entryOptions.length === 0}
                  {...register('fiveStarItemId', {
                    required: i18n.t('Pages.Gacha.LegacyView.Toolbar.Url.ManualInsertDialog.FiveStarName.Required'),
                  })}
                >
                  <option value="">
                    {loadingEntryOptions
                      ? i18n.t('Pages.Gacha.LegacyView.Toolbar.Url.ManualInsertDialog.FiveStarName.Loading')
                      : entryOptions.length === 0
                        ? i18n.t('Pages.Gacha.LegacyView.Toolbar.Url.ManualInsertDialog.FiveStarName.Empty')
                        : i18n.t('Pages.Gacha.LegacyView.Toolbar.Url.ManualInsertDialog.FiveStarName.Placeholder')
                    }
                  </option>
                  {entryOptions.map((option) => (
                    <option key={option.itemId} value={option.itemId}>
                      {`${option.name} · ${option.itemType}`}
                    </option>
                  ))}
                </Select>
              </Field>
              <Field
                size="large"
                label={<Locale mapping={['Pages.Gacha.LegacyView.Toolbar.Url.ManualInsertDialog.UpBanner.Label']} />}
              >
                <Select
                  appearance="filled-darker"
                  value={selectedUpBannerIndex}
                  disabled={isSubmitting || !selectedEntry || selectedEntry.upBanners.length === 0}
                  onChange={handleUpBannerSelect}
                >
                  <option value="">
                    {!selectedEntry
                      ? i18n.t('Pages.Gacha.LegacyView.Toolbar.Url.ManualInsertDialog.UpBanner.Placeholder')
                      : selectedEntry.upBanners.length === 0
                        ? i18n.t('Pages.Gacha.LegacyView.Toolbar.Url.ManualInsertDialog.UpBanner.Empty')
                        : i18n.t('Pages.Gacha.LegacyView.Toolbar.Url.ManualInsertDialog.UpBanner.Placeholder')
                    }
                  </option>
                  {selectedEntry?.upBanners.map((banner, index) => (
                    <option key={`${banner.startTime}_${banner.endTime}_${index}`} value={index}>
                      {(banner.version ?? '-') + ' | ' + dayjs(banner.startTime).format('YYYY-MM-DD HH:mm') + ' ~ ' + dayjs(banner.endTime).format('YYYY-MM-DD HH:mm')}
                    </option>
                  ))}
                </Select>
                <div className={styles.hint}>
                  <Locale mapping={['Pages.Gacha.LegacyView.Toolbar.Url.ManualInsertDialog.UpBanner.Help']} />
                </div>
              </Field>
              <Field
                size="large"
                validationState={errors.pullCount ? 'error' : isValid ? 'success' : 'none'}
                validationMessage={errors.pullCount?.message}
                label={<Locale mapping={['Pages.Gacha.LegacyView.Toolbar.Url.ManualInsertDialog.PullCount.Label']} />}
                required
              >
                <Input
                  type="number"
                  min={1}
                  max={5000}
                  appearance="filled-darker"
                  autoComplete="off"
                  disabled={isSubmitting}
                  placeholder={i18n.t('Pages.Gacha.LegacyView.Toolbar.Url.ManualInsertDialog.PullCount.Placeholder')}
                  {...register('pullCount', {
                    required: i18n.t('Pages.Gacha.LegacyView.Toolbar.Url.ManualInsertDialog.PullCount.Required'),
                    validate: (value) => {
                      const parsed = Number.parseInt(value, 10)
                      if (!Number.isSafeInteger(parsed) || parsed < 1) {
                        return i18n.t('Pages.Gacha.LegacyView.Toolbar.Url.ManualInsertDialog.PullCount.ValidateMin')
                      }

                      if (parsed > 5000) {
                        return i18n.t('Pages.Gacha.LegacyView.Toolbar.Url.ManualInsertDialog.PullCount.ValidateMax')
                      }
                    },
                  })}
                />
              </Field>
              <Field
                size="large"
                validationState={errors.endTime ? 'error' : isValid ? 'success' : 'none'}
                validationMessage={errors.endTime?.message}
                label={<Locale mapping={['Pages.Gacha.LegacyView.Toolbar.Url.ManualInsertDialog.EndTime.Label']} />}
                required
              >
                <Input
                  type="datetime-local"
                  step={1}
                  appearance="filled-darker"
                  disabled={isSubmitting}
                  {...register('endTime', {
                    required: i18n.t('Pages.Gacha.LegacyView.Toolbar.Url.ManualInsertDialog.EndTime.Required'),
                    validate: (value) => {
                      if (!dayjs(value).isValid()) {
                        return i18n.t('Pages.Gacha.LegacyView.Toolbar.Url.ManualInsertDialog.EndTime.Validate')
                      }
                    },
                  })}
                />
              </Field>
              <div className={styles.actions}>
                <Locale
                  component={Button}
                  appearance="secondary"
                  disabled={isSubmitting}
                  onClick={close}
                  mapping={['Pages.Gacha.LegacyView.Toolbar.Url.ManualInsertDialog.CancelBtn']}
                />
                <Locale
                  component={Button}
                  appearance="primary"
                  type="submit"
                  disabled={!isValid || isSubmitting || !selectedAccount}
                  mapping={['Pages.Gacha.LegacyView.Toolbar.Url.ManualInsertDialog.SubmitBtn']}
                />
              </div>
            </form>
          </DialogContent>
        </DialogBody>
      </DialogSurface>
    </Dialog>
  )
})

export default ManualInsertDialog
