/** Message text from the core, with its `backticked` names shown as code. */
export function Rich({ text }: { text: string }) {
  const parts = text.split(/`([^`]+)`/)
  return (
    <>
      {parts.map((part, index) =>
        index % 2 === 1 ? (
          <code key={index} className="code">
            {part}
          </code>
        ) : (
          part
        )
      )}
    </>
  )
}
