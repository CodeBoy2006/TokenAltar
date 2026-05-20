type ErrorPayload = {
  error?: unknown
}

export async function apiRequest<T>(path: string, token: string, options: RequestInit = {}): Promise<T> {
  const response = await fetch(`/api${path}`, {
    ...options,
    headers: {
      'content-type': 'application/json',
      ...(token ? { authorization: `Bearer ${token}` } : {}),
      ...(options.headers || {}),
    },
  })
  const text = await response.text()
  const data = parseJsonResponse(text)
  if (!response.ok) {
    throw new Error(errorMessage(data, response.statusText))
  }
  return data as T
}

function parseJsonResponse(text: string): unknown {
  if (!text) return null
  try {
    return JSON.parse(text)
  } catch {
    throw new Error('API returned invalid JSON')
  }
}

function errorMessage(data: unknown, fallback: string) {
  if (data && typeof data === 'object' && 'error' in data) {
    const { error } = data as ErrorPayload
    if (typeof error === 'string' && error) return error
  }
  return fallback
}
