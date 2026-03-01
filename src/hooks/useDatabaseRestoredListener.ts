import { useEffect } from 'react'
import { listen } from '@tauri-apps/api/event'
import queryClient from '@/queryClient'

const EVENT_DATABASE_RESTORED = 'HG_DATABASE_RESTORED'

export default function useDatabaseRestoredListener () {
  useEffect(() => {
    let unlisten: (() => void) | undefined

    ;(async () => {
      unlisten = await listen(EVENT_DATABASE_RESTORED, () => {
        console.info('Database restored, invalidating all queries...')
        // Invalidate all queries to refresh data
        queryClient.invalidateQueries()
      })
    })()

    return () => unlisten?.()
  }, [])
}
