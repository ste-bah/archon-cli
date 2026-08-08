import { useRef, useState } from "react";
import { FileUp, Upload } from "lucide-react";
import { StatusPill } from "../../components/StatusPill";
import { uploadFiles } from "./uploadClient";

interface UploadPanelProps {
  enabled: boolean;
  policyReason: string;
  /** Called with the stored server-side path once a file has landed. */
  onUploaded: (path: string) => void;
}

/**
 * File picker and drop zone for the Ingest tab.
 *
 * Ingest already accepted a path, which is fine when the file is on the machine
 * running archon and tedious when it is on the machine running the browser —
 * you had to copy it across by some other route first, then type where it went.
 * This uploads it and hands the resulting path to the ingest form, so the
 * existing pipeline runs unchanged: the upload is a way of *producing* a path,
 * not a second ingest route.
 */
export function UploadPanel({ enabled, policyReason, onUploaded }: UploadPanelProps) {
  const inputRef = useRef<HTMLInputElement | null>(null);
  const [dragging, setDragging] = useState(false);
  const [note, setNote] = useState("");
  const [busy, setBusy] = useState(false);

  const accept = async (files: File[]) => {
    if (!enabled || files.length === 0) return;
    setBusy(true);
    setNote(`uploading ${files.length === 1 ? files[0]?.name : `${files.length} files`}…`);
    const result = await uploadFiles(files);
    setBusy(false);
    if (result.error) {
      setNote(result.error);
      return;
    }
    const last = result.files[result.files.length - 1];
    if (last) {
      onUploaded(last.path);
      setNote(
        result.files.length === 1
          ? `${last.fileName} ready — path filled in below`
          : `${result.files.length} files uploaded; last path filled in below`,
      );
    }
  };

  return (
    <div className="panel ingest-upload">
      <div className="panel-heading">
        <div>
          <span className="eyebrow">Upload</span>
          <h3>Add a file from this machine</h3>
        </div>
        <StatusPill tone={enabled ? "good" : "warn"}>
          {enabled ? "enabled" : "policy blocked"}
        </StatusPill>
      </div>
      <div
        className={
          dragging ? "ingest-dropzone ingest-dropzone--active" : "ingest-dropzone"
        }
        onDragOver={(event) => {
          // Without preventDefault the browser navigates to the dropped file
          // and the page is gone.
          event.preventDefault();
          if (enabled) setDragging(true);
        }}
        onDragLeave={() => setDragging(false)}
        onDrop={(event) => {
          event.preventDefault();
          setDragging(false);
          void accept(Array.from(event.dataTransfer.files));
        }}
      >
        <FileUp size={22} aria-hidden="true" />
        <p>{enabled ? "Drop a file here, or" : "File uploads are disabled by policy"}</p>
        <button
          type="button"
          disabled={!enabled || busy}
          onClick={() => inputRef.current?.click()}
        >
          <Upload size={14} aria-hidden="true" /> Choose file
        </button>
        <input
          ref={inputRef}
          type="file"
          multiple
          hidden
          onChange={(event) => {
            void accept(Array.from(event.target.files ?? []));
            // Reset so choosing the same file twice fires change again.
            event.target.value = "";
          }}
        />
      </div>
      <p className="ingest-upload-note">{note || policyReason}</p>
    </div>
  );
}
