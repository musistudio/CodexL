import {
  CheckCircle2,
  Edit3,
  Monitor,
  Plus,
  ScanLine,
  Search,
  Trash2,
  Unplug,
  X,
} from "lucide-react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { type ReactNode, forwardRef, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Badge } from "../../../src/components/ui/badge";
import { Button } from "../../../src/components/ui/button";
import { Card, CardContent, CardFooter, CardHeader } from "../../../src/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "../../../src/components/ui/dialog";
import { Input } from "../../../src/components/ui/input";
import { Label } from "../../../src/components/ui/label";
import { cn } from "../../../src/lib/utils";
import { decodeCodexQrFromVideo } from "../qrDecoder.js";

const INSTANCE_STORAGE_KEY = "codexl-remote.instances";
const CONTROL_CONNECTION_STORAGE_PREFIX = "codexl-remote.control-connection.";
const REMOTE_MODE_WEB = "web";
const PWA_BUILD = "20260603-readable-device-workspace-name-v2";
const SERVICE_WORKER_URL = `service-worker.js?v=${PWA_BUILD}`;
const SPRING_TRANSITION = { damping: 30, mass: 0.72, stiffness: 430, type: "spring" } as const;
const SOFT_SPRING_TRANSITION = { damping: 34, mass: 0.85, stiffness: 300, type: "spring" } as const;
const QUICK_TRANSITION = { duration: 0.16, ease: [0.22, 1, 0.36, 1] } as const;
const INSTANT_TRANSITION = { duration: 0 } as const;

type Connection = {
  cloudUser?: string;
  deviceName?: string;
  jwt?: string;
  mode?: string;
  name?: string;
  remoteMode?: string;
  token?: string;
  url: string;
  webAssetBaseUrl?: string;
  webAssetVersion?: string;
  workspaceName?: string;
};

type RemoteInstance = {
  createdAt: number;
  host: string;
  id: string;
  lastConnectedAt: number;
  name: string;
  remoteMode: string;
  status: string;
  token: string;
  updatedAt: number;
  url: string;
  webAssetBaseUrl?: string;
  webAssetVersion?: string;
};

type NativeQrDetector = {
  detect(video: HTMLVideoElement): Promise<Array<{ rawValue?: string }>>;
};

type QrDetector = { detector: NativeQrDetector; type: "native" } | { type: "codex" };

export function RemoteControlListApp() {
  const reduceMotion = Boolean(useReducedMotion());
  const [instances, setInstances] = useState<RemoteInstance[]>(() => readStoredInstances());
  const [searchQuery, setSearchQuery] = useState("");
  const [mobileSearchOpen, setMobileSearchOpen] = useState(false);
  const [addOpen, setAddOpen] = useState(false);
  const [addLocked, setAddLocked] = useState(false);
  const [addName, setAddName] = useState("");
  const [connectionText, setConnectionText] = useState("");
  const [scanStatus, setScanStatus] = useState("");
  const [scanning, setScanning] = useState(false);
  const [editInstance, setEditInstance] = useState<RemoteInstance | null>(null);
  const [editName, setEditName] = useState("");
  const [editConnectionText, setEditConnectionText] = useState("");
  const [editStatus, setEditStatus] = useState("");
  const [deleteInstance, setDeleteInstance] = useState<RemoteInstance | null>(null);

  const scanStreamRef = useRef<MediaStream | null>(null);
  const scanTimerRef = useRef<number | null>(null);
  const scanVideoRef = useRef<HTMLVideoElement | null>(null);
  const addNameRef = useRef("");

  useEffect(() => {
    addNameRef.current = addName;
  }, [addName]);

  const persistInstances = useCallback((nextInstances: RemoteInstance[]) => {
    setInstances(nextInstances);
    saveStoredInstances(nextInstances);
  }, []);

  const addInstanceFromConnection = useCallback(
    (connection: Connection, { connect = false, name = "" } = {}) => {
      const result = upsertInstanceFromConnection(instancesFromStorage(), connection, {
        name,
        status: connect ? "Connecting" : "Not connected",
      });
      if (!result.instance) {
        setScanStatus("Paste a valid connection URL or QR payload.");
        return null;
      }

      persistInstances(result.instances);
      if (connect) {
        navigateToControl(result.instance);
        return result.instance;
      }

      stopQrScan();
      setAddOpen(false);
      setAddLocked(false);
      setAddName("");
      setConnectionText("");
      return result.instance;
    },
    [persistInstances],
  );

  const scanFrame = useCallback(
    async (detector: QrDetector) => {
      if (!scanStreamRef.current) {
        return;
      }

      try {
        const rawValue = await readQrRawValue(detector, scanVideoRef.current);
        const connection = parseConnection(rawValue);
        if (connection) {
          setScanStatus("QR code detected. Adding instance...");
          addInstanceFromConnection(connection, { name: addNameRef.current });
          return;
        }
      } catch {
        // Some browsers throw while the video element is warming up.
      }

      scanTimerRef.current = window.setTimeout(() => {
        void scanFrame(detector);
      }, 180);
    },
    [addInstanceFromConnection],
  );

  const startQrScan = useCallback(async () => {
    if (!window.isSecureContext && location.hostname !== "localhost" && location.hostname !== "127.0.0.1") {
      setScanStatus("Camera scanning requires HTTPS or localhost.");
      return;
    }
    if (!navigator.mediaDevices?.getUserMedia) {
      setScanStatus("Camera access is not available in this browser.");
      return;
    }

    try {
      stopQrScan();
      setScanStatus("Requesting camera permission...");
      const detector = createQrDetector();
      const stream = await navigator.mediaDevices.getUserMedia({
        audio: false,
        video: {
          facingMode: { ideal: "environment" },
          height: { ideal: 1080 },
          width: { ideal: 1080 },
        },
      });
      scanStreamRef.current = stream;
      setScanning(true);

      if (scanVideoRef.current) {
        scanVideoRef.current.srcObject = stream;
        await scanVideoRef.current.play();
      }
      setScanStatus("Point the camera at the CodexL QR code.");
      void scanFrame(detector);
    } catch {
      stopQrScan();
      setScanStatus("Camera permission was denied or the camera is unavailable.");
    }
  }, [scanFrame]);

  function stopQrScan() {
    if (scanTimerRef.current) {
      clearTimeout(scanTimerRef.current);
      scanTimerRef.current = null;
    }
    if (scanStreamRef.current) {
      for (const track of scanStreamRef.current.getTracks()) {
        track.stop();
      }
      scanStreamRef.current = null;
    }
    if (scanVideoRef.current) {
      scanVideoRef.current.pause();
      scanVideoRef.current.srcObject = null;
    }
    setScanning(false);
  }

  useEffect(() => {
    let cancelled = false;

    const initialize = async () => {
      registerServiceWorker();
      const resetResult = resetTransientInstanceStatuses(readStoredInstances());
      if (resetResult.changed) {
        saveStoredInstances(resetResult.instances);
      }

      const urlParams = new URLSearchParams(location.search);
      const initialConnection = connectionFromUrlParams(urlParams);
      if (initialConnection) {
        const result = upsertInstanceFromConnection(resetResult.instances, initialConnection, {
          status: "Not connected",
        });
        if (result.instance) {
          persistInstances(result.instances);
          if (shouldAddOnlyFromUrlParams(urlParams)) {
            replaceListUrlWithoutConnectionParams();
            return;
          }
          navigateToControl(result.instance);
          return;
        }
      }

      const restoredConnection = await connectionFromRemoteInfoCookie();
      if (cancelled) {
        return;
      }
      if (restoredConnection) {
        const result = upsertInstanceFromConnection(resetResult.instances, restoredConnection, {
          status: "Not connected",
        });
        if (result.instance) {
          persistInstances(result.instances);
          return;
        }
      }

      setInstances(resetResult.instances);
      if (resetResult.instances.length === 0) {
        setAddLocked(true);
        setAddOpen(true);
      }
    };

    void initialize();
    return () => {
      cancelled = true;
    };
  }, [persistInstances]);

  useEffect(() => () => stopQrScan(), []);

  const filteredInstances = useMemo(() => {
    const query = normalizeSearchQuery(searchQuery);
    if (!query) {
      return instances;
    }

    return instances.filter((instance) => instanceSearchText(instance).includes(query));
  }, [instances, searchQuery]);

  const closeMobileSearch = () => {
    setMobileSearchOpen(false);
    if (searchQuery) {
      setSearchQuery("");
    }
  };

  const openAddDialog = () => {
    setAddLocked(false);
    setAddName("");
    setConnectionText("");
    setScanStatus("");
    setAddOpen(true);
  };

  const saveManualConnection = () => {
    const connection = parseConnection(connectionText);
    if (!connection) {
      setScanStatus("Paste a valid connection URL or QR payload.");
      return;
    }
    addInstanceFromConnection(connection, { name: addName });
  };

  const openEditDialog = (instance: RemoteInstance) => {
    setEditInstance(instance);
    setEditName(instance.name || "");
    setEditConnectionText(instance.url || "");
    setEditStatus("");
  };

  const saveEdit = () => {
    if (!editInstance) {
      return;
    }

    const connection = parseConnection(editConnectionText);
    if (!connection) {
      setEditStatus("Connection URL is invalid.");
      return;
    }

    const next = buildInstanceFromConnection(connection, {
      existing: editInstance,
      name: editName,
      status: editInstance.status,
    });
    if (!next) {
      setEditStatus("Connection token is missing.");
      return;
    }

    const duplicate = instances.find(
      (instance) => instance.id !== editInstance.id && instanceIdentity(instance) === instanceIdentity(next),
    );
    if (duplicate) {
      setEditStatus("Another instance already uses this connection.");
      return;
    }

    persistInstances(instances.map((instance) => (instance.id === editInstance.id ? next : instance)));
    setEditInstance(null);
  };

  const confirmDelete = () => {
    if (!deleteInstance) {
      return;
    }

    const nextInstances = instances.filter((instance) => instance.id !== deleteInstance.id);
    persistInstances(nextInstances);
    if (editInstance?.id === deleteInstance.id) {
      setEditInstance(null);
    }
    setDeleteInstance(null);
    if (nextInstances.length === 0) {
      setAddLocked(true);
      setAddOpen(true);
    }
  };

  return (
    <motion.main
      animate={{ opacity: 1 }}
      className="flex h-full flex-col overflow-hidden bg-background text-foreground"
      initial={{ opacity: 0 }}
      style={{
        background:
          "radial-gradient(circle at 20% 0%, rgba(79, 180, 119, 0.14), transparent 28rem), var(--pwa-background)",
        padding: "calc(18px + env(safe-area-inset-top)) 18px calc(18px + env(safe-area-inset-bottom))",
      }}
      transition={motionTransition(reduceMotion, QUICK_TRANSITION)}
    >
      <section className="mx-auto flex min-h-0 w-full max-w-[920px] flex-1 flex-col">
        <header className="shrink-0 pb-3">
          <div className="flex flex-wrap items-center gap-2 sm:flex-nowrap sm:gap-3">
            <motion.div
              animate={{ opacity: 1, y: 0 }}
              aria-label="CodexL"
              className="flex min-w-0 shrink-0 items-center gap-2.5"
              initial={{ opacity: 0, y: reduceMotion ? 0 : -6 }}
              transition={motionTransition(reduceMotion, SOFT_SPRING_TRANSITION)}
            >
              <img className="h-8 w-8 shrink-0 rounded-md" src="icon.png" alt="" />
              <h1 className="m-0 text-[22px] font-bold leading-none">CodexL</h1>
            </motion.div>

            <Input
              aria-label="Search instances"
              className="hidden h-10 bg-[#0f1115] text-base sm:block sm:min-w-0 sm:max-w-[420px] sm:flex-1"
              onChange={(event) => setSearchQuery(event.target.value)}
              placeholder="Search instances"
              type="search"
              value={searchQuery}
            />

            <div className="ml-auto flex shrink-0 gap-2">
              <MotionButtonFrame reduceMotion={reduceMotion}>
                <Button
                  aria-expanded={mobileSearchOpen}
                  aria-label={mobileSearchOpen ? "Close search" : "Search instances"}
                  className="h-10 w-10 p-0 sm:hidden"
                  onClick={() => {
                    if (mobileSearchOpen) {
                      closeMobileSearch();
                      return;
                    }
                    setMobileSearchOpen(true);
                  }}
                  title={mobileSearchOpen ? "Close search" : "Search instances"}
                  type="button"
                  variant="secondary"
                >
                  <AnimatePresence initial={false} mode="wait">
                    <motion.span
                      animate={{ opacity: 1, rotate: 0, scale: 1 }}
                      className="inline-flex"
                      exit={{ opacity: 0, rotate: reduceMotion ? 0 : -18, scale: reduceMotion ? 1 : 0.86 }}
                      initial={{ opacity: 0, rotate: reduceMotion ? 0 : 18, scale: reduceMotion ? 1 : 0.86 }}
                      key={mobileSearchOpen ? "close" : "search"}
                      transition={motionTransition(reduceMotion, SPRING_TRANSITION)}
                    >
                      {mobileSearchOpen ? <X className="h-5 w-5" /> : <Search className="h-5 w-5" />}
                    </motion.span>
                  </AnimatePresence>
                </Button>
              </MotionButtonFrame>

              {instances.length > 0 ? (
                <MotionButtonFrame reduceMotion={reduceMotion}>
                  <Button
                    aria-label="Add instance"
                    className="h-10 w-10 p-0 sm:w-auto sm:px-4"
                    onClick={openAddDialog}
                    title="Add instance"
                    type="button"
                  >
                    <Plus className="h-5 w-5" />
                    <span className="hidden sm:inline">Add</span>
                  </Button>
                </MotionButtonFrame>
              ) : null}
            </div>

            <AnimatePresence initial={false}>
              {mobileSearchOpen ? (
                <motion.div
                  animate={{ height: "auto", opacity: 1, y: 0 }}
                  className="order-3 basis-full overflow-hidden sm:hidden"
                  exit={{ height: 0, opacity: 0, y: reduceMotion ? 0 : -8 }}
                  initial={{ height: 0, opacity: 0, y: reduceMotion ? 0 : -8 }}
                  transition={motionTransition(reduceMotion, SOFT_SPRING_TRANSITION)}
                >
                  <Input
                    aria-label="Search instances"
                    className="h-10 w-full bg-[#0f1115] text-base"
                    onChange={(event) => setSearchQuery(event.target.value)}
                    placeholder="Search instances"
                    type="search"
                    value={searchQuery}
                  />
                </motion.div>
              ) : null}
            </AnimatePresence>
          </div>
        </header>

        <motion.div
          className="grid min-h-0 flex-1 auto-rows-max grid-cols-[repeat(auto-fit,minmax(min(100%,280px),1fr))] content-start items-start gap-3 overflow-auto pb-6 [-webkit-overflow-scrolling:touch] [overscroll-behavior:contain]"
          layout={!reduceMotion}
          transition={motionTransition(reduceMotion, SOFT_SPRING_TRANSITION)}
        >
          <AnimatePresence initial={false} mode="popLayout">
            {instances.length > 0 && filteredInstances.length === 0 ? (
              <motion.div
                animate={{ opacity: 1, scale: 1, y: 0 }}
                className="flex min-h-28 items-center justify-center rounded-md border border-dashed border-muted-foreground/30 p-5 text-center text-sm text-muted-foreground"
                exit={{ opacity: 0, scale: reduceMotion ? 1 : 0.98, y: reduceMotion ? 0 : -8 }}
                initial={{ opacity: 0, scale: reduceMotion ? 1 : 0.98, y: reduceMotion ? 0 : 10 }}
                key="empty-search"
                layout={!reduceMotion}
                transition={motionTransition(reduceMotion, SOFT_SPRING_TRANSITION)}
              >
                No instances match your search.
              </motion.div>
            ) : null}

            {filteredInstances.map((instance) => (
              <InstanceCard
                instance={instance}
                key={instance.id}
                onConnect={() => navigateToControl(instance)}
                onDelete={() => setDeleteInstance(instance)}
                onEdit={() => openEditDialog(instance)}
                reduceMotion={reduceMotion}
              />
            ))}
          </AnimatePresence>
        </motion.div>
      </section>

      <Dialog
        open={addOpen}
        onOpenChange={(open) => {
          if (addLocked && !open) {
            return;
          }
          if (!open) {
            stopQrScan();
          }
          setAddOpen(open);
        }}
      >
        <DialogContent className="max-h-[calc(100dvh-32px)] overflow-auto" showCloseButton={!addLocked}>
          <MotionDialogPanel reduceMotion={reduceMotion}>
            <DialogHeader>
              <DialogTitle>Add instance</DialogTitle>
              <DialogDescription>Scan the QR code shown in CodexL, or paste the connection URL.</DialogDescription>
            </DialogHeader>

            <div className="grid gap-3">
              <Label htmlFor="instanceNameInput">Name</Label>
              <Input
                id="instanceNameInput"
                onChange={(event) => setAddName(event.target.value)}
                placeholder="Office Mac"
                value={addName}
              />

              <AnimatePresence initial={false}>
                {scanStatus ? (
                  <motion.div
                    animate={{ opacity: 1, y: 0 }}
                    className="rounded-md border border-border bg-white/5 p-3 text-sm text-muted-foreground"
                    exit={{ opacity: 0, y: reduceMotion ? 0 : -6 }}
                    initial={{ opacity: 0, y: reduceMotion ? 0 : 6 }}
                    key={scanStatus}
                    transition={motionTransition(reduceMotion, QUICK_TRANSITION)}
                  >
                    {scanStatus}
                  </motion.div>
                ) : null}
              </AnimatePresence>
              <motion.video
                animate={{
                  opacity: scanning ? 1 : 0,
                  scale: scanning || reduceMotion ? 1 : 0.98,
                }}
                autoPlay
                className={cn(
                  "aspect-square w-full rounded-lg border border-border bg-black object-cover",
                  !scanning && "pointer-events-none h-0 border-transparent",
                )}
                initial={false}
                muted
                playsInline
                ref={scanVideoRef}
                transition={motionTransition(reduceMotion, SOFT_SPRING_TRANSITION)}
              />
              <Label htmlFor="connectionInput">Connection URL</Label>
              <textarea
                className="min-h-24 rounded-md border border-input bg-background px-3 py-2 text-base text-foreground shadow-none placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                id="connectionInput"
                onChange={(event) => setConnectionText(event.target.value)}
                placeholder="http://192.168.1.10:3147/?token=..."
                spellCheck={false}
                value={connectionText}
              />
            </div>

            <div className="grid grid-cols-2 gap-2">
              <MotionButtonFrame className="min-w-0" reduceMotion={reduceMotion}>
                <Button className="w-full" onClick={() => void startQrScan()} type="button" variant="secondary">
                  <ScanLine className="h-4 w-4" />
                  Scan QR
                </Button>
              </MotionButtonFrame>
              <MotionButtonFrame className="min-w-0" reduceMotion={reduceMotion}>
                <Button className="w-full" onClick={saveManualConnection} type="button">
                  Add
                </Button>
              </MotionButtonFrame>
            </div>
          </MotionDialogPanel>
        </DialogContent>
      </Dialog>

      <Dialog open={Boolean(editInstance)} onOpenChange={(open) => !open && setEditInstance(null)}>
        <DialogContent>
          <MotionDialogPanel reduceMotion={reduceMotion}>
            <DialogHeader>
              <p className="text-xs font-bold text-primary">CodexLR</p>
              <DialogTitle>Edit instance</DialogTitle>
            </DialogHeader>

            <div className="grid gap-3">
              <Label htmlFor="editNameInput">Name</Label>
              <Input id="editNameInput" onChange={(event) => setEditName(event.target.value)} value={editName} />
              <Label htmlFor="editConnectionInput">Connection URL</Label>
              <textarea
                className="min-h-24 rounded-md border border-input bg-background px-3 py-2 text-base text-foreground shadow-none focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                id="editConnectionInput"
                onChange={(event) => setEditConnectionText(event.target.value)}
                spellCheck={false}
                value={editConnectionText}
              />
              <AnimatePresence initial={false}>
                {editStatus ? (
                  <motion.div
                    animate={{ opacity: 1, y: 0 }}
                    className="rounded-md border border-border bg-white/5 p-3 text-sm text-muted-foreground"
                    exit={{ opacity: 0, y: reduceMotion ? 0 : -6 }}
                    initial={{ opacity: 0, y: reduceMotion ? 0 : 6 }}
                    transition={motionTransition(reduceMotion, QUICK_TRANSITION)}
                  >
                    {editStatus}
                  </motion.div>
                ) : null}
              </AnimatePresence>
            </div>

            <DialogFooter>
              {editInstance ? (
                <MotionButtonFrame reduceMotion={reduceMotion}>
                  <Button className="w-full sm:w-auto" onClick={() => setDeleteInstance(editInstance)} type="button" variant="dangerOutline">
                    <Trash2 className="h-4 w-4" />
                    Delete
                  </Button>
                </MotionButtonFrame>
              ) : null}
              <MotionButtonFrame reduceMotion={reduceMotion}>
                <Button className="w-full sm:w-auto" onClick={saveEdit} type="button">
                  Save
                </Button>
              </MotionButtonFrame>
            </DialogFooter>
          </MotionDialogPanel>
        </DialogContent>
      </Dialog>

      <Dialog open={Boolean(deleteInstance)} onOpenChange={(open) => !open && setDeleteInstance(null)}>
        <DialogContent className="max-w-sm" showCloseButton={false}>
          <MotionDialogPanel reduceMotion={reduceMotion}>
            <DialogHeader>
              <p className="text-xs font-bold text-primary">CodexLR</p>
              <DialogTitle>Delete instance</DialogTitle>
              <DialogDescription>
                Delete "{deleteInstance?.name || "Untitled instance"}"? This instance will be removed from the list.
              </DialogDescription>
            </DialogHeader>
            <div className="grid grid-cols-2 gap-2">
              <MotionButtonFrame className="min-w-0" reduceMotion={reduceMotion}>
                <Button className="w-full" onClick={() => setDeleteInstance(null)} type="button" variant="secondary">
                  Cancel
                </Button>
              </MotionButtonFrame>
              <MotionButtonFrame className="min-w-0" reduceMotion={reduceMotion}>
                <Button className="w-full" onClick={confirmDelete} type="button" variant="dangerOutline">
                  Delete
                </Button>
              </MotionButtonFrame>
            </div>
          </MotionDialogPanel>
        </DialogContent>
      </Dialog>
    </motion.main>
  );
}

type InstanceCardProps = {
  instance: RemoteInstance;
  onConnect: () => void;
  onDelete: () => void;
  onEdit: () => void;
  reduceMotion: boolean;
};

const InstanceCard = forwardRef<HTMLDivElement, InstanceCardProps>(function InstanceCard(
  { instance, onConnect, onDelete, onEdit, reduceMotion },
  ref,
) {
  const status = instance.status || "Not connected";

  return (
    <motion.div
      animate={{ opacity: 1, scale: 1, y: 0 }}
      exit={{ opacity: 0, scale: reduceMotion ? 1 : 0.96, y: reduceMotion ? 0 : -12 }}
      initial={{ opacity: 0, scale: reduceMotion ? 1 : 0.97, y: reduceMotion ? 0 : 18 }}
      layout={!reduceMotion}
      ref={ref}
      transition={motionTransition(reduceMotion, SPRING_TRANSITION)}
      whileHover={reduceMotion ? undefined : { y: -2 }}
    >
      <Card className="flex min-h-[152px] flex-col gap-3 rounded-md bg-card/95 p-4">
        <CardHeader className="grid grid-cols-[minmax(0,1fr)_auto] gap-2 p-0">
          <div className="min-w-0">
            <h2 className="m-0 text-lg font-bold leading-tight [overflow-wrap:anywhere]">
              {instance.name || "Untitled instance"}
            </h2>
            <p className="mt-1.5 text-sm text-muted-foreground [overflow-wrap:anywhere]">
              {instance.host || hostFromConnectionUrl(instance.url) || "Unknown host"}
            </p>
          </div>
          <StatusBadge reduceMotion={reduceMotion} status={status} />
        </CardHeader>

        <CardContent className="grid gap-1 p-0 text-xs leading-relaxed text-muted-foreground">
          <div>Mode: {remoteModeLabel(instance.remoteMode)}</div>
          {instance.webAssetBaseUrl ? <div>Bundle: {instance.webAssetVersion || "latest"}</div> : null}
          <div>Last connected: {formatTime(instance.lastConnectedAt)}</div>
        </CardContent>

        <CardFooter className="mt-auto grid grid-cols-[minmax(0,1.3fr)_minmax(0,1fr)_minmax(0,1fr)] gap-2 p-0">
          <MotionButtonFrame reduceMotion={reduceMotion}>
            <Button className="w-full" onClick={onConnect} type="button">
              <Monitor className="h-4 w-4" />
              Connect
            </Button>
          </MotionButtonFrame>
          <MotionButtonFrame reduceMotion={reduceMotion}>
            <Button className="w-full" onClick={onEdit} type="button" variant="secondary">
              <Edit3 className="h-4 w-4" />
              Edit
            </Button>
          </MotionButtonFrame>
          <MotionButtonFrame reduceMotion={reduceMotion}>
            <Button className="w-full" onClick={onDelete} type="button" variant="dangerOutline">
              <Trash2 className="h-4 w-4" />
              Delete
            </Button>
          </MotionButtonFrame>
        </CardFooter>
      </Card>
    </motion.div>
  );
});

function StatusBadge({ reduceMotion, status }: { reduceMotion: boolean; status: string }) {
  const kind = statusKind(status);
  return (
    <AnimatePresence initial={false} mode="wait">
      <motion.span
        animate={{ opacity: 1, scale: 1, x: 0 }}
        className="inline-flex justify-end"
        exit={{ opacity: 0, scale: reduceMotion ? 1 : 0.92, x: reduceMotion ? 0 : 8 }}
        initial={{ opacity: 0, scale: reduceMotion ? 1 : 0.92, x: reduceMotion ? 0 : 8 }}
        key={`${kind}-${status}`}
        transition={motionTransition(reduceMotion, SPRING_TRANSITION)}
      >
        {kind === "connected" || kind === "cdp" ? (
          <Badge className="max-w-[44vw] overflow-hidden text-ellipsis whitespace-nowrap" variant="success">
            <CheckCircle2 className="h-3.5 w-3.5" />
            {status}
          </Badge>
        ) : null}
        {kind === "connecting" || kind === "retrying" ? (
          <Badge className="max-w-[44vw] overflow-hidden text-ellipsis whitespace-nowrap border border-amber-300/30 bg-amber-300/10 text-amber-200" variant="secondary">
            {status}
          </Badge>
        ) : null}
        {kind === "idle" ? (
          <Badge className="max-w-[44vw] overflow-hidden text-ellipsis whitespace-nowrap border border-muted-foreground/30 bg-muted text-muted-foreground" variant="secondary">
            <Unplug className="h-3.5 w-3.5" />
            {status || "Not connected"}
          </Badge>
        ) : null}
      </motion.span>
    </AnimatePresence>
  );
}

function MotionButtonFrame({
  children,
  className,
  reduceMotion,
}: {
  children: ReactNode;
  className?: string;
  reduceMotion: boolean;
}) {
  return (
    <motion.div
      className={className}
      transition={motionTransition(reduceMotion, SPRING_TRANSITION)}
      whileHover={reduceMotion ? undefined : { y: -1 }}
      whileTap={reduceMotion ? undefined : { scale: 0.96, y: 1 }}
    >
      {children}
    </motion.div>
  );
}

function MotionDialogPanel({ children, reduceMotion }: { children: ReactNode; reduceMotion: boolean }) {
  return (
    <motion.div
      animate={{ opacity: 1, scale: 1, y: 0 }}
      className="grid gap-4"
      initial={{ opacity: 0, scale: reduceMotion ? 1 : 0.96, y: reduceMotion ? 0 : 14 }}
      transition={motionTransition(reduceMotion, SPRING_TRANSITION)}
    >
      {children}
    </motion.div>
  );
}

function motionTransition<T>(reduceMotion: boolean, transition: T) {
  return reduceMotion ? INSTANT_TRANSITION : transition;
}

function instancesFromStorage(): RemoteInstance[] {
  return readStoredInstances();
}

function readStoredInstances(): RemoteInstance[] {
  try {
    const raw = localStorage.getItem(INSTANCE_STORAGE_KEY);
    const stored = raw ? JSON.parse(raw) : [];
    if (!Array.isArray(stored)) {
      return [];
    }

    return stored.map(normalizeStoredInstance).filter(Boolean) as RemoteInstance[];
  } catch {
    return [];
  }
}

function normalizeStoredInstance(instance: unknown): RemoteInstance | null {
  if (!instance || typeof instance !== "object") {
    return null;
  }
  const stored = instance as Partial<RemoteInstance>;

  return buildInstanceFromConnection(
    {
      remoteMode: typeof stored.remoteMode === "string" ? stored.remoteMode : "",
      token: typeof stored.token === "string" ? stored.token : "",
      url: typeof stored.url === "string" ? stored.url : "",
      webAssetBaseUrl: typeof stored.webAssetBaseUrl === "string" ? stored.webAssetBaseUrl : "",
      webAssetVersion: typeof stored.webAssetVersion === "string" ? stored.webAssetVersion : "",
    },
    {
      existing: {
        createdAt: Number(stored.createdAt) || Date.now(),
        host: typeof stored.host === "string" ? stored.host : "",
        id: typeof stored.id === "string" && stored.id ? stored.id : createInstanceId(),
        lastConnectedAt: Number(stored.lastConnectedAt) || 0,
        name: typeof stored.name === "string" ? stored.name : "",
        remoteMode: typeof stored.remoteMode === "string" ? stored.remoteMode : "",
        status: typeof stored.status === "string" && stored.status ? stored.status : "Not connected",
        token: typeof stored.token === "string" ? stored.token : "",
        updatedAt: Number(stored.updatedAt) || Date.now(),
        url: typeof stored.url === "string" ? stored.url : "",
      },
      name: typeof stored.name === "string" ? stored.name : "",
      status: typeof stored.status === "string" && stored.status ? stored.status : "Not connected",
    },
  );
}

function saveStoredInstances(instances: RemoteInstance[]) {
  try {
    localStorage.setItem(INSTANCE_STORAGE_KEY, JSON.stringify(instances));
  } catch {
    // Storage can be blocked in private browsing modes.
  }
}

function resetTransientInstanceStatuses(instances: RemoteInstance[]) {
  let changed = false;
  const nextInstances = instances.map((instance) => {
    if (statusKind(instance.status) === "idle") {
      return instance;
    }

    changed = true;
    return {
      ...instance,
      status: "Not connected",
      updatedAt: Date.now(),
    };
  });

  return { changed, instances: nextInstances };
}

function upsertInstanceFromConnection(
  instances: RemoteInstance[],
  connection: Connection,
  { name = "", status = "" } = {},
): { instance: RemoteInstance | null; instances: RemoteInstance[] } {
  const candidate = buildInstanceFromConnection(connection, { name, status });
  if (!candidate) {
    return { instance: null, instances };
  }

  const identity = instanceIdentity(candidate);
  const existing = instances.find((instance) => instanceIdentity(instance) === identity);
  if (existing) {
    const suggestedName = suggestedInstanceName(connection);
    const updated = {
      ...existing,
      host: candidate.host,
      name: normalizeInstanceName(name) || suggestedName || existing.name || candidate.name,
      remoteMode: candidate.remoteMode,
      status: status || existing.status,
      token: candidate.token,
      updatedAt: Date.now(),
      url: candidate.url,
      webAssetBaseUrl: candidate.webAssetBaseUrl,
      webAssetVersion: candidate.webAssetVersion,
    };
    return {
      instance: updated,
      instances: instances.map((instance) => (instance.id === existing.id ? updated : instance)),
    };
  }

  return { instance: candidate, instances: [candidate, ...instances] };
}

function buildInstanceFromConnection(
  connection: Connection,
  { existing = null, name = "", status = "" }: { existing?: RemoteInstance | null; name?: string; status?: string } = {},
): RemoteInstance | null {
  let connectionUrl: URL;
  try {
    connectionUrl = normalizeConnectionUrl(connection.url);
  } catch {
    return null;
  }

  const nextToken = connection.token || connectionUrl.searchParams.get("token") || "";
  if (!nextToken) {
    return null;
  }
  if (connection.cloudUser) {
    connectionUrl.searchParams.set("cloudUser", connection.cloudUser);
  }
  if (connection.jwt) {
    connectionUrl.searchParams.set("jwt", connection.jwt);
  }
  const nextWebAssetBaseUrl = connectionWebAssetBaseUrl(connection, connectionUrl);
  const nextWebAssetVersion = connectionWebAssetVersion(connection, connectionUrl);
  if (nextWebAssetBaseUrl) {
    connectionUrl.searchParams.set("webAssetBaseUrl", nextWebAssetBaseUrl);
    connectionUrl.searchParams.set("webAssetVersion", nextWebAssetVersion);
  }
  if (!connectionUsesCloudRemote(connection, connectionUrl)) {
    connectionUrl.searchParams.delete("requirePassword");
    connectionUrl.searchParams.delete("e2ee");
  }

  const now = Date.now();
  const remoteMode = normalizeRemoteMode(
    connection.remoteMode ||
      connection.mode ||
      connectionUrl.searchParams.get("remoteMode") ||
      connectionUrl.searchParams.get("mode") ||
      existing?.remoteMode,
  );
  const suggestedName = suggestedInstanceName(connection, connectionUrl);
  if (suggestedName) {
    connectionUrl.searchParams.set("name", suggestedName);
  }
  const displayName = normalizeInstanceName(name) || existing?.name || suggestedName || defaultInstanceName(connectionUrl);

  return {
    createdAt: existing?.createdAt || now,
    host: connectionUrl.host,
    id: existing?.id || createInstanceId(),
    lastConnectedAt: existing?.lastConnectedAt || 0,
    name: displayName,
    remoteMode,
    status: status || existing?.status || "Not connected",
    token: nextToken,
    updatedAt: now,
    url: connectionUrl.toString(),
    webAssetBaseUrl: nextWebAssetBaseUrl,
    webAssetVersion: nextWebAssetVersion,
  };
}

function instanceIdentity(instance: RemoteInstance) {
  let url: URL;
  try {
    url = normalizeConnectionUrl(instance.url);
  } catch {
    return `${instance.host || ""}|${instance.token || ""}`;
  }

  return `${url.origin}${websocketBasePath(url.pathname)}|${instance.token || ""}`;
}

function createInstanceId() {
  if (globalThis.crypto?.randomUUID) {
    return globalThis.crypto.randomUUID();
  }

  return `instance-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

function normalizeInstanceName(name: string | undefined) {
  return String(name || "").trim().replace(/\s+/g, " ");
}

function suggestedInstanceName(connection: Connection, parsedUrl?: URL) {
  let connectionUrl = parsedUrl;
  if (!connectionUrl) {
    try {
      connectionUrl = normalizeConnectionUrl(connection.url);
    } catch {
      connectionUrl = undefined;
    }
  }

  const directName = normalizeInstanceName(
    connection.name ||
      connectionUrl?.searchParams.get("name") ||
      connectionUrl?.searchParams.get("itemName") ||
      connectionUrl?.searchParams.get("item_name") ||
      "",
  );
  if (directName) {
    return directName;
  }

  return mobileInstanceItemName(
    connection.deviceName || connectionUrl?.searchParams.get("deviceName") || connectionUrl?.searchParams.get("device_name") || "",
    connection.workspaceName ||
      connectionUrl?.searchParams.get("workspaceName") ||
      connectionUrl?.searchParams.get("workspace_name") ||
      "",
  );
}

function mobileInstanceItemName(deviceName: string | undefined, workspaceName: string | undefined) {
  const device = mobileInstanceNameSegment(deviceName);
  const workspace = mobileInstanceNameSegment(workspaceName);
  return [device, workspace].filter(Boolean).join("-");
}

function mobileInstanceNameSegment(value: string | undefined) {
  return String(value || "")
    .trim()
    .replace(/\.local$/i, "")
    .toLowerCase()
    .replace(/[\/\\-]+/g, "-")
    .replace(/[\u0000-\u001f\u007f]+/g, " ")
    .replace(/\s+/g, " ")
    .replace(/\s*-\s*/g, "-")
    .replace(/-+/g, "-")
    .replace(/^-+|-+$/g, "")
    .trim();
}

function defaultInstanceName(url: URL) {
  return url.hostname;
}

function hostFromConnectionUrl(value: string) {
  try {
    return normalizeConnectionUrl(value).host;
  } catch {
    return "";
  }
}

function statusKind(status: string) {
  const normalized = String(status || "").toLowerCase();
  if (!normalized || normalized === "not connected") {
    return "idle";
  }
  if (normalized.includes("disconnect") || normalized.includes("retry")) {
    return "retrying";
  }
  if (normalized.includes("connecting") || normalized.includes("loading") || normalized.includes("preparing")) {
    return "connecting";
  }
  if (normalized.includes("cdp connected")) {
    return "cdp";
  }
  if (normalized.includes("web connected")) {
    return "connected";
  }
  if (normalized.includes("connected")) {
    return "connected";
  }
  return "idle";
}

function formatTime(value: number) {
  const timestamp = Number(value) || 0;
  if (!timestamp) {
    return "Never";
  }

  return new Intl.DateTimeFormat(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(new Date(timestamp));
}

function connectionFromUrlParams(params: URLSearchParams): Connection | null {
  const encodedUrl = params.get("url") || params.get("connection") || "";
  if (encodedUrl) {
    const parsed = parseConnection(encodedUrl);
    if (parsed) {
      return parsed;
    }
  }

  const directToken = params.get("token") || "";
  if (!directToken) {
    return null;
  }

  return {
    cloudUser: params.get("cloudUser") || "",
    deviceName: params.get("deviceName") || params.get("device_name") || "",
    jwt: params.get("jwt") || "",
    remoteMode: params.get("remoteMode") || params.get("mode") || "",
    name: params.get("name") || params.get("itemName") || params.get("item_name") || "",
    token: directToken,
    url: location.href,
    webAssetBaseUrl: params.get("webAssetBaseUrl") || params.get("web_asset_base_url") || "",
    webAssetVersion: params.get("webAssetVersion") || params.get("web_asset_version") || "",
    workspaceName: params.get("workspaceName") || params.get("workspace_name") || "",
  };
}

async function connectionFromRemoteInfoCookie(): Promise<Connection | null> {
  try {
    const infoUrl = new URL(`${websocketBasePath(location.pathname)}/api/remote-info`, location.origin);
    const controller = new AbortController();
    const timeout = window.setTimeout(() => controller.abort(), 1800);
    const response = await fetch(infoUrl, {
      cache: "no-store",
      credentials: "same-origin",
      signal: controller.signal,
    }).finally(() => {
      window.clearTimeout(timeout);
    });
    if (!response.ok) {
      return null;
    }
    const info = (await response.json()) as Record<string, unknown>;
    if (!info || typeof info !== "object") {
      return null;
    }
    return {
      cloudUser: remoteInfoField(info, "cloudUserId", "cloud_user_id") || "",
      deviceName: remoteInfoField(info, "deviceName", "device_name") || "",
      jwt: "",
      remoteMode: remoteInfoField(info, "remoteMode", "remote_mode", "mode") || "",
      name: remoteInfoField(info, "name", "itemName", "item_name") || "",
      token: remoteInfoField(info, "token") || "",
      url: remoteInfoField(info, "lanUrl", "lan_url", "url") || location.href,
      webAssetBaseUrl: remoteInfoField(info, "webAssetBaseUrl", "web_asset_base_url") || "",
      webAssetVersion: remoteInfoField(info, "webAssetVersion", "web_asset_version") || "",
      workspaceName: remoteInfoField(info, "workspaceName", "workspace_name") || "",
    };
  } catch {
    return null;
  }
}

function remoteInfoField(info: Record<string, unknown>, ...keys: string[]) {
  for (const key of keys) {
    const value = info[key];
    if (typeof value === "string" && value.trim()) {
      return value;
    }
  }
  return "";
}

function shouldAddOnlyFromUrlParams(params: URLSearchParams) {
  const value = params.get("addOnly") || params.get("add_only") || "";
  return value === "1" || value.toLowerCase() === "true";
}

function replaceListUrlWithoutConnectionParams() {
  try {
    const url = new URL("index.html", location.href);
    history.replaceState(null, "", url.toString());
  } catch {
    // The connection is already stored; keeping the original URL is still usable.
  }
}

function parseConnection(raw: string): Connection | null {
  const value = String(raw || "").trim();
  if (!value) {
    return null;
  }

  try {
    const parsed = JSON.parse(value) as Record<string, unknown>;
    if (parsed && typeof parsed === "object") {
      if (typeof parsed.url === "string") {
        const connection = parseConnection(parsed.url);
        if (!connection) {
          return null;
        }
        return {
          cloudUser: typeof parsed.cloudUser === "string" ? parsed.cloudUser : connection.cloudUser,
          deviceName:
            typeof parsed.deviceName === "string"
              ? parsed.deviceName
              : typeof parsed.device_name === "string"
                ? parsed.device_name
                : connection.deviceName,
          jwt: typeof parsed.jwt === "string" ? parsed.jwt : connection.jwt,
          remoteMode:
            typeof parsed.remoteMode === "string"
              ? parsed.remoteMode
              : typeof parsed.mode === "string"
                ? parsed.mode
                : connection.remoteMode,
          name:
            typeof parsed.name === "string"
              ? parsed.name
              : typeof parsed.itemName === "string"
                ? parsed.itemName
                : typeof parsed.item_name === "string"
                  ? parsed.item_name
                  : connection.name,
          token: typeof parsed.token === "string" ? parsed.token : connection.token,
          url: connection.url,
          webAssetBaseUrl:
            typeof parsed.webAssetBaseUrl === "string"
              ? parsed.webAssetBaseUrl
              : typeof parsed.web_asset_base_url === "string"
                ? parsed.web_asset_base_url
                : connection.webAssetBaseUrl,
          webAssetVersion:
            typeof parsed.webAssetVersion === "string"
              ? parsed.webAssetVersion
              : typeof parsed.web_asset_version === "string"
                ? parsed.web_asset_version
                : connection.webAssetVersion,
          workspaceName:
            typeof parsed.workspaceName === "string"
              ? parsed.workspaceName
              : typeof parsed.workspace_name === "string"
                ? parsed.workspace_name
                : connection.workspaceName,
        };
      }

      if (typeof parsed.host === "string" && parsed.port) {
        const protocol = typeof parsed.protocol === "string" ? parsed.protocol.replace(/:$/, "") : "http";
        const url = new URL(`${protocol}://${parsed.host}:${parsed.port}/`);
        if (typeof parsed.token === "string") {
          url.searchParams.set("token", parsed.token);
        }
        if (typeof parsed.cloudUser === "string") {
          url.searchParams.set("cloudUser", parsed.cloudUser);
        }
        if (typeof parsed.jwt === "string") {
          url.searchParams.set("jwt", parsed.jwt);
        }
        const parsedName =
          typeof parsed.name === "string"
            ? parsed.name
            : typeof parsed.itemName === "string"
              ? parsed.itemName
              : typeof parsed.item_name === "string"
                ? parsed.item_name
                : "";
        if (parsedName) {
          url.searchParams.set("name", parsedName);
        }
        const parsedDeviceName =
          typeof parsed.deviceName === "string"
            ? parsed.deviceName
            : typeof parsed.device_name === "string"
              ? parsed.device_name
              : "";
        if (parsedDeviceName) {
          url.searchParams.set("deviceName", parsedDeviceName);
        }
        const parsedWorkspaceName =
          typeof parsed.workspaceName === "string"
            ? parsed.workspaceName
            : typeof parsed.workspace_name === "string"
              ? parsed.workspace_name
              : "";
        if (parsedWorkspaceName) {
          url.searchParams.set("workspaceName", parsedWorkspaceName);
        }
        const parsedMode =
          typeof parsed.remoteMode === "string"
            ? parsed.remoteMode
            : typeof parsed.mode === "string"
              ? parsed.mode
              : "";
        if (parsedMode) {
          url.searchParams.set("remoteMode", parsedMode);
        }
        const parsedWebAssetBaseUrl =
          typeof parsed.webAssetBaseUrl === "string"
            ? parsed.webAssetBaseUrl
            : typeof parsed.web_asset_base_url === "string"
              ? parsed.web_asset_base_url
              : "";
        const parsedWebAssetVersion =
          typeof parsed.webAssetVersion === "string"
            ? parsed.webAssetVersion
            : typeof parsed.web_asset_version === "string"
              ? parsed.web_asset_version
              : "";
        if (parsedWebAssetBaseUrl) {
          url.searchParams.set("webAssetBaseUrl", parsedWebAssetBaseUrl);
          url.searchParams.set("webAssetVersion", parsedWebAssetVersion || "latest");
        }
        return {
          cloudUser: typeof parsed.cloudUser === "string" ? parsed.cloudUser : "",
          deviceName: parsedDeviceName,
          jwt: typeof parsed.jwt === "string" ? parsed.jwt : "",
          remoteMode: parsedMode,
          name: parsedName,
          token: typeof parsed.token === "string" ? parsed.token : "",
          url: url.toString(),
          webAssetBaseUrl: parsedWebAssetBaseUrl,
          webAssetVersion: parsedWebAssetVersion,
          workspaceName: parsedWorkspaceName,
        };
      }
    }
  } catch {
    // Treat non-JSON payloads as URLs below.
  }

  try {
    const url = normalizeConnectionUrl(value);
    return {
      cloudUser: url.searchParams.get("cloudUser") || "",
      deviceName: url.searchParams.get("deviceName") || url.searchParams.get("device_name") || "",
      jwt: url.searchParams.get("jwt") || "",
      remoteMode: url.searchParams.get("remoteMode") || url.searchParams.get("mode") || "",
      name: url.searchParams.get("name") || url.searchParams.get("itemName") || url.searchParams.get("item_name") || "",
      token: url.searchParams.get("token") || "",
      url: url.toString(),
      webAssetBaseUrl: url.searchParams.get("webAssetBaseUrl") || url.searchParams.get("web_asset_base_url") || "",
      webAssetVersion: url.searchParams.get("webAssetVersion") || url.searchParams.get("web_asset_version") || "",
      workspaceName: url.searchParams.get("workspaceName") || url.searchParams.get("workspace_name") || "",
    };
  } catch {
    return null;
  }
}

function normalizeConnectionUrl(value: string) {
  const url = new URL(String(value || "").trim());
  if (url.protocol !== "http:" && url.protocol !== "https:") {
    throw new Error("Unsupported connection protocol");
  }
  return url;
}

function connectionWebAssetBaseUrl(connection: Connection, connectionUrl: URL) {
  return normalizeWebAssetBaseUrl(
    connection.webAssetBaseUrl ||
      connectionUrl.searchParams.get("webAssetBaseUrl") ||
      connectionUrl.searchParams.get("web_asset_base_url") ||
      "",
  );
}

function connectionWebAssetVersion(connection: Connection, connectionUrl: URL) {
  return normalizeWebAssetVersion(
    connection.webAssetVersion ||
      connectionUrl.searchParams.get("webAssetVersion") ||
      connectionUrl.searchParams.get("web_asset_version") ||
      "latest",
  );
}

function connectionUsesCloudRemote(connection: Connection | RemoteInstance, connectionUrl: URL) {
  const candidate = connection as Connection & {
    auth?: string;
    authMode?: string;
    auth_mode?: string;
    cloud_user?: string;
  };
  const authMode = String(candidate.auth || candidate.authMode || candidate.auth_mode || connectionUrl.searchParams.get("auth") || "")
    .trim()
    .toLowerCase();
  return (
    authMode === "cloud" ||
    Boolean(candidate.cloudUser || candidate.cloud_user || candidate.jwt || connectionUrl.searchParams.get("cloudUser") || connectionUrl.searchParams.get("jwt"))
  );
}

function normalizeWebAssetBaseUrl(value: string | undefined) {
  const raw = String(value || "").trim();
  if (!raw) {
    return "";
  }
  try {
    const url = new URL(raw, location.href);
    if (url.protocol !== "http:" && url.protocol !== "https:") {
      return "";
    }
    url.hash = "";
    url.search = "";
    return url.toString().replace(/\/+$/, "");
  } catch {
    return "";
  }
}

function normalizeWebAssetVersion(value: string | undefined) {
  const version = String(value || "").trim() || "latest";
  return /^[0-9A-Za-z._-]+$/.test(version) ? version : "latest";
}

function normalizeRemoteMode(_mode: string | undefined) {
  return REMOTE_MODE_WEB;
}

function remoteModeLabel(mode: string) {
  return normalizeRemoteMode(mode) === REMOTE_MODE_WEB ? "Web" : "Screen";
}

function websocketBasePath(pathname: string) {
  if (pathname === "/" || pathname === "/index.html") {
    return "";
  }

  if (pathname.endsWith("/index.html")) {
    return pathname.slice(0, -"/index.html".length);
  }

  return pathname.endsWith("/") ? pathname.slice(0, -1) : "";
}

function navigateToControl(instance: RemoteInstance) {
  const url = new URL("control.html", location.href);
  const connectionUrl = connectionUrlForNavigation(instance);
  url.searchParams.set("id", instance.id);
  url.searchParams.set("url", connectionUrl);
  applyControlConnectionParams(url.searchParams, instance, connectionUrl);
  rememberControlConnection(instance);
  location.href = url.toString();
}

function connectionUrlForNavigation(instance: RemoteInstance) {
  try {
    const url = normalizeConnectionUrl(instance.url);
    if (instance.token) {
      url.searchParams.set("token", instance.token);
    }
    if (!connectionUsesCloudRemote(instance, url)) {
      url.searchParams.delete("requirePassword");
      url.searchParams.delete("e2ee");
    }
    return url.toString();
  } catch {
    return instance.url || "";
  }
}

function applyControlConnectionParams(params: URLSearchParams, instance: RemoteInstance, connectionUrl: string) {
  if (instance.token) {
    params.set("token", instance.token);
  }
  if (instance.remoteMode) {
    params.set("remoteMode", instance.remoteMode);
  }
  if (instance.webAssetBaseUrl) {
    params.set("webAssetBaseUrl", instance.webAssetBaseUrl);
  }
  if (instance.webAssetVersion) {
    params.set("webAssetVersion", instance.webAssetVersion);
  }
  try {
    const url = normalizeConnectionUrl(connectionUrl);
    for (const key of ["cloudUser", "jwt", "transport"]) {
      const value = url.searchParams.get(key);
      if (value) {
        params.set(key, value);
      }
    }
    if (connectionUsesCloudRemote(instance, url)) {
      for (const key of ["requirePassword", "e2ee"]) {
        const value = url.searchParams.get(key);
        if (value) {
          params.set(key, value);
        }
      }
    }
  } catch {
    // The nested connection URL remains the primary source when it is parseable.
  }
}

function rememberControlConnection(instance: RemoteInstance) {
  try {
    sessionStorage.setItem(controlConnectionStorageKey(instance.id), JSON.stringify(instance));
  } catch {
    // URL parameters still carry the connection for reloads when session storage is unavailable.
  }
}

function controlConnectionStorageKey(instanceId: string) {
  return `${CONTROL_CONNECTION_STORAGE_PREFIX}${instanceId}`;
}

function normalizeSearchQuery(value: string) {
  return String(value || "").trim().toLowerCase();
}

function instanceSearchText(instance: RemoteInstance) {
  return normalizeSearchQuery(
    [
      instance.name,
      instance.host,
      hostFromConnectionUrl(instance.url),
      remoteModeLabel(instance.remoteMode),
      instance.webAssetVersion || "",
      instance.status || "Not connected",
    ].join(" "),
  );
}

function createQrDetector(): QrDetector {
  const BarcodeDetectorCtor = (
    window as unknown as {
      BarcodeDetector?: new (options: { formats: string[] }) => NativeQrDetector;
    }
  ).BarcodeDetector;
  if (BarcodeDetectorCtor) {
    try {
      return { detector: new BarcodeDetectorCtor({ formats: ["qr_code"] }), type: "native" };
    } catch {
      // Fall through to the local CodexL QR decoder.
    }
  }
  return { type: "codex" };
}

async function readQrRawValue(detector: QrDetector, video: HTMLVideoElement | null) {
  if (!video) {
    return "";
  }
  if (detector.type === "native") {
    try {
      const codes = await detector.detector.detect(video);
      const nativeValue = codes?.[0]?.rawValue || "";
      if (nativeValue) {
        return nativeValue;
      }
    } catch {
      // Some browsers expose BarcodeDetector but fail on live video frames.
    }
  }
  return decodeCodexQrFromVideo(video) || "";
}

function registerServiceWorker() {
  if (!("serviceWorker" in navigator) || location.protocol === "file:") {
    return;
  }
  navigator.serviceWorker.addEventListener("controllerchange", () => {
    if (sessionStorage.getItem("codexl-sw-reloaded") === PWA_BUILD) {
      return;
    }
    sessionStorage.setItem("codexl-sw-reloaded", PWA_BUILD);
    location.reload();
  });
  navigator.serviceWorker
    .register(SERVICE_WORKER_URL)
    .then((registration) => registration.update())
    .catch(() => {
      // The list remains usable without offline caching.
    });
}
