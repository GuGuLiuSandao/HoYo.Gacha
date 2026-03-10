import { ChangeEvent, forwardRef, useCallback, useEffect, useImperativeHandle, useMemo, useRef, useState } from 'react'
import { SubmitHandler, useForm, useWatch } from 'react-hook-form'
import { Button, Dialog, DialogBody, DialogContent, DialogSurface, DialogTitle, Field, Input, Select, makeStyles, tokens } from '@fluentui/react-components'
import { produce } from 'immer'
import BusinessCommands, { ManualInsertEntryOption, ManualInsertGachaEntryOptionsArgs, ManualInsertGachaRecordsArgs } from '@/api/commands/business'
import errorTrans from '@/api/errorTrans'
import { Account, AccountBusiness, KeyofAccountBusiness } from '@/api/schemas/Account'
import useDialogOpenEffect from '@/hooks/useDialogOpenEffect'
import { WithTransKnownNs, useI18n } from '@/i18n'
import { useUpdateAccountPropertiesMutation } from '@/pages/Gacha/queries/accounts'
import { invalidatePrettizedRecordsQuery } from '@/pages/Gacha/queries/prettizedRecords'
import useAppNotifier, { DefaultNotifierTimeouts } from '@/pages/Root/hooks/useAppNotifier'
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

const ManualInsertGachaTypeOptions: Record<number, {
  value: number
  categoryKey: string
  suffix?: string
}[]> = {
  [AccountBusiness.GenshinImpact]: [
    { value: 100, categoryKey: 'Beginner' },
    { value: 200, categoryKey: 'Permanent' },
    { value: 301, categoryKey: 'Character', suffix: '1' },
    { value: 400, categoryKey: 'Character', suffix: '2' },
    { value: 302, categoryKey: 'Weapon' },
    { value: 500, categoryKey: 'Chronicled' },
  ],
  [AccountBusiness.HonkaiStarRail]: [
    { value: 2, categoryKey: 'Beginner' },
    { value: 1, categoryKey: 'Permanent' },
    { value: 11, categoryKey: 'Character', suffix: '1' },
    { value: 12, categoryKey: 'CollaborationCharacter' },
    { value: 21, categoryKey: 'Weapon', suffix: '1' },
    { value: 22, categoryKey: 'CollaborationWeapon' },
  ],
  [AccountBusiness.ZenlessZoneZero]: [
    { value: 3, categoryKey: 'Permanent' },
    { value: 1, categoryKey: 'Character' },
    { value: 102, categoryKey: 'ExclusiveRescreening' },
    { value: 2, categoryKey: 'Weapon' },
    { value: 103, categoryKey: 'WEngineReverberation' },
    { value: 5, categoryKey: 'Bangboo' },
  ],
}

function defaultFormValues (business: AccountBusiness) {
  return {
    gachaType: String(ManualInsertGachaTypeOptions[business]?.[0]?.value ?? ''),
    fiveStarItemId: '',
    pullCount: '1',
    endTime: dayjs().format('YYYY-MM-DDTHH:mm:ss'),
  }
}

interface FormData {
  gachaType: string
  fiveStarItemId: string
  pullCount: string
  endTime: string
}

type ManualInsertGachaType = ManualInsertGachaRecordsArgs<AccountBusiness>['gachaType']

function isManualInsertGachaType (
  value: number,
  options: { value: number }[],
): value is ManualInsertGachaType {
  return options.some((option) => option.value === value)
}

export interface ManualInsertDialogProps {
  business: AccountBusiness
  owner: Account
  onCancel?: () => void
  onSuccess?: () => void
}

function ManualInsertForm (props: ManualInsertDialogProps) {
  const { business, owner, onCancel, onSuccess } = props
  const styles = useStyles()
  const { t, constants } = useI18n(WithTransKnownNs.GachaPage)
  const gachaLocale = constants.gacha
  const keyofBusiness = AccountBusiness[business] as KeyofAccountBusiness
  const notifier = useAppNotifier()
  const i18nRef = useRef(t)
  const notifierRef = useRef(notifier)
  const [entryOptions, setEntryOptions] = useState<ManualInsertEntryOption[]>([])
  const [loadingEntryOptions, setLoadingEntryOptions] = useState(false)
  const [selectedUpBannerIndex, setSelectedUpBannerIndex] = useState('')
  const gachaTypeOptions = useMemo(
    () => ManualInsertGachaTypeOptions[business] ?? [],
    [business],
  )

  const updateAccountPropertiesMutation = useUpdateAccountPropertiesMutation()

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
  const selectedEntry = useMemo(
    () => entryOptions.find((entry) => String(entry.itemId) === watchedItemId) ?? null,
    [entryOptions, watchedItemId],
  )

  useEffect(() => {
    i18nRef.current = t
    notifierRef.current = notifier
  }, [notifier, t])

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

    const args: ManualInsertGachaEntryOptionsArgs<AccountBusiness> = {
      business,
      gachaType,
      customLocale: gachaLocale,
    }

    const loadEntryOptions = async () => {
      try {
        const entries = await BusinessCommands.manualInsertGachaEntryOptions(args)
        if (disposed) return

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
        if (disposed) return

        setEntryOptions([])
        setSelectedUpBannerIndex('')
        setValue('fiveStarItemId', '', {
          shouldValidate: true,
        })
        notifierRef.current.error(
          i18nRef.current('Toolbar.GachaUrl.ManualInsert.LoadEntryOptionsError', { keyof: keyofBusiness }),
          {
            body: errorTrans(i18nRef.current, error),
            timeout: DefaultNotifierTimeouts.error * 2,
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
  }, [business, gachaLocale, gachaTypeOptions, getValues, keyofBusiness, setValue, watchedGachaType])

  const handleUpBannerSelect = useCallback((event: ChangeEvent<HTMLSelectElement>) => {
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

  const handleConfirm = useCallback<SubmitHandler<FormData>>(async (data) => {
    const pullCount = Number.parseInt(data.pullCount, 10)
    if (!Number.isSafeInteger(pullCount) || pullCount <= 0) {
      setError('pullCount', {
        message: t('Toolbar.GachaUrl.ManualInsertDialog.PullCount.ValidateMin'),
      })
      return
    }

    const gachaType = Number.parseInt(data.gachaType, 10)
    if (!Number.isSafeInteger(gachaType) || !isManualInsertGachaType(gachaType, gachaTypeOptions)) {
      return
    }

    if (!selectedEntry) {
      setError('fiveStarItemId', {
        message: t('Toolbar.GachaUrl.ManualInsertDialog.FiveStarName.Required'),
      })
      return
    }

    const date = dayjs(data.endTime)
    if (!date.isValid()) {
      setError('endTime', {
        message: t('Toolbar.GachaUrl.ManualInsertDialog.EndTime.Validate'),
      })
      return
    }

    const args: ManualInsertGachaRecordsArgs<AccountBusiness> = {
      business,
      uid: owner.uid,
      gachaType,
      fiveStarName: selectedEntry.name,
      pullCount,
      endTime: date.toDate().toISOString(),
      customLocale: gachaLocale,
    }

    const changes = await notifier.promise(
      BusinessCommands.manualInsertGachaRecords(args),
      {
        loading: {
          title: t('Toolbar.GachaUrl.ManualInsert.Loading', { keyof: keyofBusiness }),
        },
        success: (result) => ({
          title: t('Toolbar.GachaUrl.ManualInsert.Success.Title', { keyof: keyofBusiness }),
          body: t('Toolbar.GachaUrl.ManualInsert.Success.Body', { changes: result }),
          timeout: DefaultNotifierTimeouts.success * 2,
          dismissible: true,
        }),
        error: (error) => ({
          title: t('Toolbar.GachaUrl.ManualInsert.Error', { keyof: keyofBusiness }),
          body: errorTrans(t, error),
          timeout: DefaultNotifierTimeouts.error * 2,
          dismissible: true,
        }),
      },
    )

    if (!changes) {
      onSuccess?.()
      return
    }

    const now = dayjs().toISOString()
    const properties = owner.properties
      ? produce(owner.properties, (draft) => {
          draft.lastGachaRecordsUpdated = now
        })
      : { lastGachaRecordsUpdated: now }

    await updateAccountPropertiesMutation.mutateAsync({
      business,
      uid: owner.uid,
      properties,
    })

    invalidatePrettizedRecordsQuery(owner.business, owner.uid, gachaLocale)
    onSuccess?.()
  }, [
    business,
    gachaLocale,
    gachaTypeOptions,
    keyofBusiness,
    notifier,
    onSuccess,
    owner.business,
    owner.properties,
    owner.uid,
    selectedEntry,
    setError,
    t,
    updateAccountPropertiesMutation,
  ])

  const fiveStarOptionPlaceholder = loadingEntryOptions
    ? t('Toolbar.GachaUrl.ManualInsertDialog.FiveStarName.Loading')
    : entryOptions.length === 0
      ? t('Toolbar.GachaUrl.ManualInsertDialog.FiveStarName.Empty')
      : t('Toolbar.GachaUrl.ManualInsertDialog.FiveStarName.Placeholder')

  const upBannerOptionPlaceholder = !selectedEntry
    ? t('Toolbar.GachaUrl.ManualInsertDialog.UpBanner.Placeholder')
    : selectedEntry.upBanners.length === 0
      ? t('Toolbar.GachaUrl.ManualInsertDialog.UpBanner.Empty')
      : t('Toolbar.GachaUrl.ManualInsertDialog.UpBanner.Placeholder')

  return (
    <form
      className={styles.form}
      onSubmit={handleSubmit(handleConfirm)}
      noValidate
    >
      <Field
        size="large"
        label={t('Toolbar.GachaUrl.ManualInsertDialog.GachaType.Label')}
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
            const title = t(`Common:${keyofBusiness}.Gacha.Category.${option.categoryKey}`)
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
        label={t('Toolbar.GachaUrl.ManualInsertDialog.FiveStarName.Label')}
        required
      >
        <Select
          appearance="filled-darker"
          disabled={isSubmitting || loadingEntryOptions || entryOptions.length === 0}
          {...register('fiveStarItemId', {
            required: t('Toolbar.GachaUrl.ManualInsertDialog.FiveStarName.Required'),
          })}
        >
          <option value="">{fiveStarOptionPlaceholder}</option>
          {entryOptions.map((option) => (
            <option key={option.itemId} value={option.itemId}>
              {`${option.name} · ${option.itemType}`}
            </option>
          ))}
        </Select>
      </Field>
      <Field
        size="large"
        label={t('Toolbar.GachaUrl.ManualInsertDialog.UpBanner.Label')}
      >
        <Select
          appearance="filled-darker"
          value={selectedUpBannerIndex}
          disabled={isSubmitting || !selectedEntry || selectedEntry.upBanners.length === 0}
          onChange={handleUpBannerSelect}
        >
          <option value="">{upBannerOptionPlaceholder}</option>
          {selectedEntry?.upBanners.map((banner, index) => (
            <option key={`${banner.startTime}_${banner.endTime}_${index}`} value={index}>
              {(banner.version ?? '-') + ' | ' + dayjs(banner.startTime).format('YYYY-MM-DD HH:mm') + ' ~ ' + dayjs(banner.endTime).format('YYYY-MM-DD HH:mm')}
            </option>
          ))}
        </Select>
        <div className={styles.hint}>
          {t('Toolbar.GachaUrl.ManualInsertDialog.UpBanner.Help')}
        </div>
      </Field>
      <Field
        size="large"
        validationState={errors.pullCount ? 'error' : isValid ? 'success' : 'none'}
        validationMessage={errors.pullCount?.message}
        label={t('Toolbar.GachaUrl.ManualInsertDialog.PullCount.Label')}
        required
      >
        <Input
          type="number"
          min={1}
          max={5000}
          appearance="filled-darker"
          autoComplete="off"
          disabled={isSubmitting}
          placeholder={t('Toolbar.GachaUrl.ManualInsertDialog.PullCount.Placeholder')}
          {...register('pullCount', {
            required: t('Toolbar.GachaUrl.ManualInsertDialog.PullCount.Required'),
            validate: (value) => {
              const parsed = Number.parseInt(value, 10)
              if (!Number.isSafeInteger(parsed) || parsed < 1) {
                return t('Toolbar.GachaUrl.ManualInsertDialog.PullCount.ValidateMin')
              }

              if (parsed > 5000) {
                return t('Toolbar.GachaUrl.ManualInsertDialog.PullCount.ValidateMax')
              }
            },
          })}
        />
      </Field>
      <Field
        size="large"
        validationState={errors.endTime ? 'error' : isValid ? 'success' : 'none'}
        validationMessage={errors.endTime?.message}
        label={t('Toolbar.GachaUrl.ManualInsertDialog.EndTime.Label')}
        required
      >
        <Input
          type="datetime-local"
          step={1}
          appearance="filled-darker"
          disabled={isSubmitting}
          {...register('endTime', {
            required: t('Toolbar.GachaUrl.ManualInsertDialog.EndTime.Required'),
            validate: (value) => {
              if (!dayjs(value).isValid()) {
                return t('Toolbar.GachaUrl.ManualInsertDialog.EndTime.Validate')
              }
            },
          })}
        />
      </Field>
      <div className={styles.actions}>
        <Button
          appearance="secondary"
          disabled={isSubmitting}
          onClick={onCancel}
        >
          {t('Toolbar.GachaUrl.ManualInsertDialog.CancelBtn')}
        </Button>
        <Button
          appearance="primary"
          type="submit"
          disabled={!isValid || isSubmitting}
        >
          {t('Toolbar.GachaUrl.ManualInsertDialog.SubmitBtn')}
        </Button>
      </div>
    </form>
  )
}

const ManualInsertDialog = forwardRef<
  { open (owner: Account): void },
  Pick<ManualInsertDialogProps, 'business'>
>(function ManualInsertDialog (props, ref) {
  const [{ owner, open }, setState] = useState<{
    owner: Account | null
    open: boolean
  }>({
    owner: null,
    open: false,
  })

  useDialogOpenEffect(open)
  useImperativeHandle(ref, () => ({
    open: (owner) => setState({
      owner,
      open: true,
    }),
  }), [])

  const { t } = useI18n(WithTransKnownNs.GachaPage)
  const close = useCallback(() => {
    setState({
      owner: null,
      open: false,
    })
  }, [])

  if (!owner) {
    return null
  }

  return (
    <Dialog open={open} modalType="alert">
      <DialogSurface>
        <DialogBody>
          <DialogTitle>
            {t('Toolbar.GachaUrl.ManualInsertDialog.Title')}
          </DialogTitle>
          <DialogContent>
            <ManualInsertForm
              business={props.business}
              owner={owner}
              onCancel={close}
              onSuccess={close}
            />
          </DialogContent>
        </DialogBody>
      </DialogSurface>
    </Dialog>
  )
})

export default ManualInsertDialog
