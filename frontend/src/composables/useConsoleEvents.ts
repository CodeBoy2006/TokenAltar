import type { ConsoleUpdateEvent } from '../api/types'

type ConsoleEventOptions = {
  getToken: () => string
  onUnauthorized: () => void
  onTopics: (topics: Set<string>) => Promise<void> | void
  onError: (error: unknown) => void
}

export function useConsoleEvents(options: ConsoleEventOptions) {
  const pendingTopics = new Set<string>()
  let eventAbort: AbortController | null = null
  let reconnectTimer: number | null = null
  let refreshTimer: number | null = null

  function start() {
    if (!options.getToken()) return
    stop()
    const controller = new AbortController()
    eventAbort = controller
    void consume(controller)
  }

  function stop() {
    if (eventAbort) {
      eventAbort.abort()
      eventAbort = null
    }
    if (reconnectTimer !== null) {
      window.clearTimeout(reconnectTimer)
      reconnectTimer = null
    }
    if (refreshTimer !== null) {
      window.clearTimeout(refreshTimer)
      refreshTimer = null
    }
    pendingTopics.clear()
  }

  async function consume(controller: AbortController) {
    try {
      const response = await fetch('/api/events', {
        headers: {
          accept: 'text/event-stream',
          authorization: `Bearer ${options.getToken()}`,
        },
        signal: controller.signal,
      })
      if (response.status === 401 || response.status === 403) {
        options.onUnauthorized()
        return
      }
      if (!response.ok || !response.body) {
        throw new Error(response.statusText || 'event stream unavailable')
      }
      const reader = response.body.getReader()
      const decoder = new TextDecoder()
      let buffer = ''
      let streamClosed = false
      while (true) {
        const { value, done } = await reader.read()
        if (done) {
          streamClosed = true
          break
        }
        buffer += decoder.decode(value, { stream: true })
        buffer = drainFrames(buffer)
      }
      buffer += decoder.decode()
      drainFrames(buffer)
      if (streamClosed && !controller.signal.aborted && options.getToken()) {
        scheduleReconnect()
      }
    } catch (err) {
      if (!controller.signal.aborted && options.getToken()) {
        scheduleReconnect()
      }
    } finally {
      if (eventAbort === controller) {
        eventAbort = null
      }
    }
  }

  function scheduleReconnect() {
    if (reconnectTimer !== null || !options.getToken()) return
    reconnectTimer = window.setTimeout(() => {
      reconnectTimer = null
      start()
    }, 2000)
  }

  function drainFrames(buffer: string) {
    const normalized = buffer.replace(/\r\n/g, '\n')
    const frames = normalized.split('\n\n')
    const remainder = frames.pop() || ''
    for (const frame of frames) {
      handleFrame(frame)
    }
    return remainder
  }

  function handleFrame(frame: string) {
    const data = frame
      .split('\n')
      .filter((line) => line.startsWith('data:'))
      .map((line) => line.slice(5).trimStart())
      .join('\n')
    if (!data) return
    try {
      const event = JSON.parse(data) as ConsoleUpdateEvent
      queueTopicRefresh(event.topics || [])
    } catch {
      queueTopicRefresh(['sync'])
    }
  }

  function queueTopicRefresh(topics: string[]) {
    for (const topic of topics) {
      if (topic !== 'connected') {
        pendingTopics.add(topic)
      }
    }
    if (pendingTopics.size === 0 || refreshTimer !== null) return
    refreshTimer = window.setTimeout(() => {
      refreshTimer = null
      void flushTopicRefresh()
    }, 250)
  }

  async function flushTopicRefresh() {
    if (!options.getToken()) return
    const topics = new Set(pendingTopics)
    pendingTopics.clear()
    try {
      await options.onTopics(topics)
    } catch (err) {
      options.onError(err)
    }
  }

  return { start, stop }
}
