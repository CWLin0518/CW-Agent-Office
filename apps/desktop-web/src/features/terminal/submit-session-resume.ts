/** Submit once to the captured session, without keyboard-protocol Escape bytes. */
export async function submitSessionResume(options: {
  prompt: string
  isCurrentSession: () => boolean
  write: (input: string) => Promise<boolean>
  sleep: (ms: number) => Promise<void>
}): Promise<boolean> {
  if (!options.isCurrentSession() || !await options.write(options.prompt)) return false
  // Allow the CLI's paste editor to commit before submitting.
  await options.sleep(500)
  if (!options.isCurrentSession()) return false
  return options.write('\r')
}
