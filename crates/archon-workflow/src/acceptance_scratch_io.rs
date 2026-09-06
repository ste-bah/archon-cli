use super::*;
use std::io::Read;

fn read(path:&Path)->WorkflowResult<Vec<u8>> {
    let mut opts=std::fs::OpenOptions::new();opts.read(true);
    #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt;opts.custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK); }
    let mut file=opts.open(path).map_err(|e|WorkflowError::io(path,e))?;
    if !file.metadata().map_err(|e|WorkflowError::io(path,e))?.is_file() {return Err(invalid("snapshot input is not a regular file"));}
    let mut bytes=Vec::new();file.read_to_end(&mut bytes).map_err(|e|WorkflowError::io(path,e))?;Ok(bytes)
}
pub(super) fn copy_tree(source:&Path,dest:&Path,remaining:&mut u64,exclude_git:bool)->WorkflowResult<()> {
    let meta=std::fs::symlink_metadata(source).map_err(|e|WorkflowError::io(source,e))?;
    if meta.is_dir() {
        if let Ok(existing)=dest.symlink_metadata() {
            if !existing.is_dir() || existing.file_type().is_symlink() {return Err(invalid("scratch input directory collision"));}
        }
        std::fs::create_dir_all(dest).map_err(|e|WorkflowError::io(dest,e))?;
        for item in std::fs::read_dir(source).map_err(|e|WorkflowError::io(source,e))? {
            let item=item.map_err(|e|WorkflowError::io(source,e))?;
            let name=item.file_name();
            if exclude_git && name==".git" {continue;}
            if !exclude_git && matches!(name.to_str(),Some("credentials"|"credentials.toml"|"config.toml"|"config.json"|".env")) {
                return Err(invalid(format!("credential/config input cannot be exported: {}",item.path().display())));
            }
            copy_tree(&item.path(),&dest.join(name),remaining,exclude_git)?;
        }
    } else if meta.is_file() {
        if meta.len()>*remaining {return Err(invalid("scratch copy exceeds configured size limit"));}
        let bytes=read(source)?;
        if let Ok(existing)=dest.symlink_metadata() {
            if !existing.is_file() || read(dest)?!=bytes {return Err(invalid(format!("nonidentical scratch path collision: {}",dest.display())));}
            return Ok(());
        }
        if let Some(parent)=dest.parent() {std::fs::create_dir_all(parent).map_err(|e|WorkflowError::io(parent,e))?;}
        std::fs::write(dest,&bytes).map_err(|e|WorkflowError::io(dest,e))?;
        std::fs::set_permissions(dest,meta.permissions()).map_err(|e|WorkflowError::io(dest,e))?;
        *remaining-=bytes.len() as u64;
    } else {return Err(invalid(format!("nonregular or symlink snapshot input refused: {}",source.display())));}
    Ok(())
}
pub(super) fn readonly(path:&Path)->WorkflowResult<()> {
    let meta=std::fs::symlink_metadata(path).map_err(|e|WorkflowError::io(path,e))?;
    if meta.is_dir() {
        for item in std::fs::read_dir(path).map_err(|e|WorkflowError::io(path,e))? {
            readonly(&item.map_err(|e|WorkflowError::io(path,e))?.path())?;
        }
    }
    let mut permissions=meta.permissions();permissions.set_readonly(true);
    std::fs::set_permissions(path,permissions).map_err(|e|WorkflowError::io(path,e))
}
pub(super) fn remove_owned_tree(path:&Path)->WorkflowResult<()> {
    let meta=match path.symlink_metadata() {Ok(m)=>m,Err(e) if e.kind()==std::io::ErrorKind::NotFound=>return Ok(()),Err(e)=>return Err(WorkflowError::io(path,e))};
    if meta.is_dir() && !meta.file_type().is_symlink() {
        #[cfg(unix)] {use std::os::unix::fs::PermissionsExt;std::fs::set_permissions(path,std::fs::Permissions::from_mode(meta.permissions().mode()|0o700)).map_err(|e|WorkflowError::io(path,e))?;}
        for item in std::fs::read_dir(path).map_err(|e|WorkflowError::io(path,e))? {remove_owned_tree(&item.map_err(|e|WorkflowError::io(path,e))?.path())?;}
        std::fs::remove_dir(path).map_err(|e|WorkflowError::io(path,e))
    } else {std::fs::remove_file(path).map_err(|e|WorkflowError::io(path,e))}
}
/// Exact byte inventory, including symbolic link targets without following them.
pub fn inventory(root:&Path)->WorkflowResult<BTreeMap<String,String>> {
    fn visit(root:&Path,path:&Path,out:&mut BTreeMap<String,String>)->WorkflowResult<()> {
        let meta=std::fs::symlink_metadata(path).map_err(|e|WorkflowError::io(path,e))?;
        let key=path.strip_prefix(root).map_err(|_|invalid("inventory escaped root"))?.to_string_lossy().into_owned();
        let value=if meta.file_type().is_symlink() {
            format!("link:{}",std::fs::read_link(path).map_err(|e|WorkflowError::io(path,e))?.display())
        } else if meta.is_dir() {"directory".into()} else if meta.is_file() {
            format!("file:{}:{}",meta.len(),crate::task_set_contract::content_digest(&read(path)?))
        } else {return Err(invalid(format!("nonregular live inventory path: {}",path.display())));};
        out.insert(key,value);
        if meta.is_dir() {
            for item in std::fs::read_dir(path).map_err(|e|WorkflowError::io(path,e))? {visit(root,&item.map_err(|e|WorkflowError::io(path,e))?.path(),out)?;}
        }
        Ok(())
    }
    let mut result=BTreeMap::new();visit(root,root,&mut result)?;Ok(result)
}
