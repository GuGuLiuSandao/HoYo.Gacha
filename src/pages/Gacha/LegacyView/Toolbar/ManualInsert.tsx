import React, { forwardRef, useCallback, useEffect, useImperativeHandle, useMemo, useState } from 'react'
import { SubmitHandler, useForm } from 'react-hook-form'
import { Button, Dialog, DialogBody, DialogContent, DialogSurface, DialogTitle, Field, Input, Select, makeStyles, tokens } from '@fluentui/react-components'
import { produce } from 'immer'
import { ManualInsertGachaRecordsArgs, manualInsertGachaRecords } from '@/api/commands/business'
import errorTranslation from '@/api/errorTranslation'
import { useSelectedAccountSuspenseQueryData, useUpdateAccountPropertiesMutation } from '@/api/queries/accounts'
import { invalidateFirstGachaRecordQuery, invalidatePrettizedGachaRecordsQuery } from '@/api/queries/business'
import Locale from '@/components/Locale'
import useI18n from '@/hooks/useI18n'
import useNotifier from '@/hooks/useNotifier'
import { Business, KeyofBusinesses, MiliastraWonderland } from '@/interfaces/Business'
import dayjs from '@/utilities/dayjs'

const useStyles = makeStyles({
  form: {
    display: 'flex',
    flexDirection: 'column',
    rowGap: tokens.spacingVerticalS,
    minWidth: '24rem',
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
  fiveStarName: '',
  pullCount: '1',
  endTime: dayjs().format('YYYY-MM-DDTHH:mm:ss'),
})

export function isManualInsertSupported (business: Business) {
  return business !== MiliastraWonderland
}

type FormData = {
  gachaType: string
  fiveStarName: string
  pullCount: string
  endTime: string
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
  const i18n = useI18n()
  const notifier = useNotifier()
  const selectedAccount = useSelectedAccountSuspenseQueryData(keyofBusinesses)
  const updateAccountPropertiesMutation = useUpdateAccountPropertiesMutation()
  const gachaTypeOptions = useMemo(
    () => ManualInsertGachaTypeOptions[business] ?? [],
    [business],
  )

  useImperativeHandle(ref, () => ({ setOpen }))

  const {
    register,
    handleSubmit,
    reset,
    setError,
    formState: { errors, isValid, isSubmitting },
  } = useForm<FormData>({
    mode: 'onChange',
    defaultValues: defaultFormValues(business),
  })

  useEffect(() => {
    reset(defaultFormValues(business))
  }, [business, reset])

  const close = useCallback(() => {
    reset(defaultFormValues(business))
    setOpen(false)
  }, [business, reset])

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
      gachaType: Number.parseInt(data.gachaType, 10),
      fiveStarName: data.fiveStarName.trim(),
      pullCount,
      endTime: date.format('YYYY-MM-DDTHH:mm:ssZ'),
      customLocale: i18n.constants.gacha,
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

    invalidatePrettizedGachaRecordsQuery(selectedAccount.business, selectedAccount.uid, i18n.constants.gacha)
    invalidateFirstGachaRecordQuery(selectedAccount.business, selectedAccount.uid)
    close()
  }, [business, close, i18n, keyofBusinesses, notifier, selectedAccount, setError, updateAccountPropertiesMutation])

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
                validationState={errors.fiveStarName ? 'error' : isValid ? 'success' : 'none'}
                validationMessage={errors.fiveStarName?.message}
                label={<Locale mapping={['Pages.Gacha.LegacyView.Toolbar.Url.ManualInsertDialog.FiveStarName.Label']} />}
                required
              >
                <Input
                  autoComplete="off"
                  appearance="filled-darker"
                  disabled={isSubmitting}
                  placeholder={i18n.t('Pages.Gacha.LegacyView.Toolbar.Url.ManualInsertDialog.FiveStarName.Placeholder')}
                  {...register('fiveStarName', {
                    required: i18n.t('Pages.Gacha.LegacyView.Toolbar.Url.ManualInsertDialog.FiveStarName.Required'),
                  })}
                />
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
