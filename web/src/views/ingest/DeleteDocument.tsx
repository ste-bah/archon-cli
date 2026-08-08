import { useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Trash2 } from "lucide-react";
import { apiClient } from "../../api/client";
import type { WebDocStoreItem } from "../../api/generated/web";

interface DeleteDocumentProps {
  document: WebDocStoreItem;
  enabled: boolean;
  policyReason: string;
}

/**
 * Delete a document, with a confirmation that names what goes.
 *
 * The counts are not decoration. Deleting a document drops its chunks, pages,
 * artifacts and raw vectors along with the source registration, and none of it
 * comes back — a confirmation that says only "are you sure?" gives the
 * operator nothing to be sure *about*. The row already carries the counts, so
 * the prompt states them.
 */
export function DeleteDocument({ document, enabled, policyReason }: DeleteDocumentProps) {
  const queryClient = useQueryClient();
  const [confirming, setConfirming] = useState(false);
  const [result, setResult] = useState("");
  const remove = useMutation({
    mutationFn: apiClient.deleteDocument,
    onSuccess: (response) => {
      setConfirming(false);
      setResult(
        response.accepted
          ? `deleted — ${response.chunks} chunks, ${response.pages} pages, ` +
              `${response.artifacts} artifacts, ${response.vectors} vectors removed`
          : response.policyReason,
      );
    },
    onError: (error: unknown) => setResult(String(error)),
    onSettled: () => queryClient.invalidateQueries({ queryKey: ["ingest"] }),
  });

  if (!enabled) {
    return <p className="ingest-delete-blocked">{policyReason}</p>;
  }

  if (result) {
    return <p className="ingest-delete-result">{result}</p>;
  }

  if (!confirming) {
    return (
      <button type="button" className="danger" onClick={() => setConfirming(true)}>
        <Trash2 size={13} aria-hidden="true" /> Delete document
      </button>
    );
  }

  return (
    <div className="ingest-delete-confirm">
      <p>
        Permanently delete <strong>{document.sourcePath}</strong>? This removes{" "}
        {document.chunks} chunks, {document.pages} pages and {document.artifacts}{" "}
        artifacts, plus its raw vectors and source registration. It cannot be undone.
      </p>
      <div>
        <button
          type="button"
          className="danger"
          disabled={remove.isPending}
          onClick={() =>
            remove.mutate({ documentId: document.documentId, confirmed: true })
          }
        >
          {remove.isPending ? "Deleting…" : "Yes, delete it"}
        </button>
        <button type="button" onClick={() => setConfirming(false)}>
          Cancel
        </button>
      </div>
    </div>
  );
}
